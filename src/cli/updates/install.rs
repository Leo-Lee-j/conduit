//! Source-aware CLI updates. Never install another copy in a different prefix.
use super::*;

const LOG_LIMIT: usize = 12 * 1024;
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);

#[derive(Debug)]
struct Plan {
    program: PathBuf,
    args: Vec<String>,
    label: &'static str,
}

fn package(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "@anthropic-ai/claude-code",
        Provider::Codex => "@openai/codex",
        Provider::OpenCode => "opencode-ai",
    }
}

fn npm_prefix(provider: Provider, path: &Path) -> Option<PathBuf> {
    let name = package(provider);
    for prefix in path.ancestors().skip(1) {
        let directory = prefix.join("node_modules").join(name);
        let manifest = directory.join("package.json");
        // A matching manifest alone must not turn an unrelated executable into npm.
        let in_package = path.starts_with(&directory);
        let shim = path.parent() == Some(prefix)
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd"))
            && path
                .file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case(provider.descriptor().id))
            && std::fs::read_to_string(path).is_ok_and(|text| {
                text.replace('\\', "/")
                    .contains(&format!("node_modules/{name}/"))
            });
        if !(in_package || shim) {
            continue;
        }
        let metadata = std::fs::read(&manifest)
            .ok()
            .filter(|bytes| bytes.len() <= 1024 * 1024)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        if metadata.as_ref().and_then(|value| value["name"].as_str()) == Some(name) {
            return Some(prefix.to_path_buf());
        }
    }
    None
}

fn search_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(name))
            .find(|path| path.is_absolute() && path.is_file())
    })
}

fn npm_program() -> Result<(PathBuf, PathBuf)> {
    let npm = search_path("npm.cmd").context(conduit::i18n::text_static(
        "cli.updates.install.npm_not_found_install_node_js_and",
    ))?;
    let directory = npm.parent().context(conduit::i18n::text_static(
        "cli.updates.install.invalid_npm_install_path",
    ))?;
    let script = directory.join("node_modules/npm/bin/npm-cli.js");
    anyhow::ensure!(
        script.is_file(),
        conduit::i18n::text("cli.updates.install.npm_cli_js_not_found_check_the")
    );
    let adjacent = directory.join("node.exe");
    let node = if adjacent.is_file() {
        adjacent
    } else {
        search_path("node.exe").context(conduit::i18n::text_static(
            "cli.updates.install.node_exe_not_found_check_the_node",
        ))?
    };
    Ok((node, script))
}

impl Plan {
    fn npm(
        provider: Provider,
        prefix: &Path,
        version: &str,
        program: PathBuf,
        script: PathBuf,
    ) -> Self {
        Self {
            program,
            args: vec![
                script.to_string_lossy().into_owned(),
                "install".into(),
                "--global".into(),
                "--prefix".into(),
                crate::child::normalized_dir(prefix)
                    .to_string_lossy()
                    .into_owned(),
                format!("{}@{version}", package(provider)),
                "--no-audit".into(),
                "--no-fund".into(),
                "--progress=false".into(),
            ],
            label: conduit::i18n::text_static("cli.updates.install.npm_original_install_directory"),
        }
    }

    fn detect(provider: Provider, path: &Path, version: &str) -> Result<Self> {
        if let Some(prefix) = npm_prefix(provider, path) {
            let other_manager = prefix.components().any(|component| {
                matches!(
                    component
                        .as_os_str()
                        .to_string_lossy()
                        .to_ascii_lowercase()
                        .as_str(),
                    "pnpm" | ".pnpm" | "yarn" | ".yarn" | ".bun"
                )
            }) || prefix.join("node_modules/.modules.yaml").exists()
                || prefix.join("node_modules/.yarn-integrity").exists();
            anyhow::ensure!(
                !other_manager,
                conduit::i18n::text(
                    "cli.updates.install.this_cli_is_managed_by_another_javascript"
                )
            );
            let (program, script) = npm_program()?;
            return Ok(Self::npm(provider, &prefix, version, program, script));
        }
        anyhow::ensure!(
            !path
                .components()
                .any(|component| component.as_os_str() == "node_modules"),
            conduit::i18n::text("cli.updates.install.unable_to_identify_the_source_of_this")
        );
        let managed = path.components().any(|component| {
            let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
            value == "scoop" || value == "chocolatey" || value.contains("winget")
        });
        anyhow::ensure!(
            !managed,
            conduit::i18n::text("cli.updates.install.this_cli_is_managed_by_a_system")
        );
        anyhow::ensure!(
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("exe")),
            conduit::i18n::text(
                "cli.updates.install.unable_to_identify_this_launcher_s_installation"
            )
        );
        let args = match provider {
            Provider::Claude => vec!["update".into()],
            Provider::OpenCode => vec!["upgrade".into(), version.into()],
            Provider::Codex => anyhow::bail!(conduit::i18n::text(
                "cli.updates.install.this_codex_installation_is_a_standalone_executable"
            )),
        };
        Ok(Self {
            program: path.to_path_buf(),
            args,
            label: conduit::i18n::text_static("cli.updates.install.native_cli_update"),
        })
    }
}

/// Run the actual update only after identifying the selected installation and checking its version.
pub(in crate::cli) fn run(
    provider: Provider,
    path: &Path,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(String),
) -> Result<VersionInfo> {
    progress(
        conduit::i18n::text_static("settings.cli.reading_local_version_and_checking_the_latest")
            .into(),
    );
    let before = read_version(provider, path, cancelled)?;
    let update = check(provider, &before.version, cancelled)?;
    if update.status != UpdateStatus::Available {
        return Ok(VersionInfo {
            update: Some(Ok(update)),
            ..before
        });
    }
    let plan = Plan::detect(provider, path, &update.latest_version)?;
    progress(conduit::i18n::format(
        "cli.updates.install.update_method_updating",
        &[
            ("arg0", plan.label.to_string()),
            ("arg1", before.version.to_string()),
            ("arg2", update.latest_version.to_string()),
        ],
    ));
    perform(provider, path, update, &plan, cancelled, &mut progress)
}

fn perform(
    provider: Provider,
    path: &Path,
    update: UpdateInfo,
    plan: &Plan,
    cancelled: &AtomicBool,
    progress: &mut impl FnMut(String),
) -> Result<VersionInfo> {
    run_command(plan, cancelled, INSTALL_TIMEOUT, progress)?;
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        conduit::i18n::text(
            "cli.updates.install.update_stopped_installation_may_be_incomplete_detect"
        )
    );
    progress(
        conduit::i18n::text_static(
            "cli.updates.install.update_command_completed_detecting_the_installed_version",
        )
        .into(),
    );
    let after = read_version(provider, path, cancelled).context(conduit::i18n::text_static(
        "cli.updates.install.update_command_completed_but_the_version_at",
    ))?;
    let current = Version::parse(&after.version).context(conduit::i18n::text_static(
        "cli.updates.install.invalid_version_format_after_update",
    ))?;
    let expected = Version::parse(&update.latest_version)?;
    anyhow::ensure!(
        current.cmp_precedence(&expected).is_ge(),
        conduit::i18n::format(
            "cli.updates.install.update_command_completed_but_version_is_still",
            &[
                ("arg0", after.version.to_string()),
                ("arg1", update.latest_version.to_string())
            ]
        )
    );
    Ok(VersionInfo {
        update: Some(Ok(UpdateInfo {
            latest_version: update.latest_version,
            status: if current.cmp_precedence(&expected).is_gt() {
                UpdateStatus::Ahead
            } else {
                UpdateStatus::Current
            },
        })),
        ..after
    })
}

fn read_version(provider: Provider, path: &Path, cancelled: &AtomicBool) -> Result<VersionInfo> {
    let output = probe_command(
        path,
        &["--version".into()],
        cancelled,
        Duration::from_secs(10),
    )?;
    parse_version(provider, &output)
}

// Read output continuously, retain a bounded tail, and throttle UI progress messages.
fn run_command(
    plan: &Plan,
    cancelled: &AtomicBool,
    timeout: Duration,
    progress: &mut impl FnMut(String),
) -> Result<()> {
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        conduit::i18n::text("cli.updates.install.update_stopped")
    );
    let environment = std::collections::BTreeMap::from([
        ("CI".into(), "true".into()),
        ("NO_COLOR".into(), "1".into()),
        ("npm_config_update_notifier".into(), "false".into()),
    ]);
    // Native executable plus structured arguments; no interpolated shell commands.
    let (child, mut pipes) = Child::spawn_with_env(
        &plan.program,
        &plan.args,
        &std::env::temp_dir(),
        &environment,
    )
    .context(conduit::i18n::text_static(
        "cli.updates.install.unable_to_start_the_update_process_check",
    ))?;
    drop(pipes.stdin);
    let mut log = Vec::new();
    let started = Instant::now();
    let mut last_progress = started;
    let result = (|| {
        loop {
            anyhow::ensure!(
                !cancelled.load(Ordering::Acquire),
                conduit::i18n::text(
                    "cli.updates.install.update_stopped_installation_may_be_incomplete_detect"
                )
            );
            anyhow::ensure!(
                started.elapsed() < timeout,
                conduit::i18n::text(
                    "cli.updates.install.update_timed_out_process_tree_stopped_installation"
                )
            );
            drain(&mut pipes.stdout, &mut log)?;
            drain(&mut pipes.stderr, &mut log)?;
            if let Some(code) = child.exit_code()? {
                drain(&mut pipes.stdout, &mut log)?;
                drain(&mut pipes.stderr, &mut log)?;
                anyhow::ensure!(
                    code == 0,
                    conduit::i18n::format(
                        "cli.updates.install.update_exit_code",
                        &[
                            ("code", format!("{}", code)),
                            ("arg0", visible_log(&log).to_string())
                        ]
                    )
                );
                return Ok(());
            }
            if last_progress.elapsed() >= Duration::from_millis(250) {
                let output = visible_log(&log);
                progress(conduit::i18n::format(
                    "cli.updates.install.updating_elapsed_seconds",
                    &[
                        ("arg0", plan.label.to_string()),
                        ("arg1", format!("{}", started.elapsed().as_secs())),
                        ("output", output.to_string()),
                    ],
                ));
                last_progress = Instant::now();
            }
            thread::sleep(Duration::from_millis(25));
        }
    })();
    child.terminate()?;
    let cleanup = Instant::now();
    while child.active_processes()? > 0 || child.exit_code()?.is_none() {
        anyhow::ensure!(
            cleanup.elapsed() < Duration::from_secs(10),
            conduit::i18n::text("cli.updates.install.update_process_cleanup_timed_out")
        );
        thread::sleep(Duration::from_millis(10));
    }
    result
}

fn drain(file: &mut File, log: &mut Vec<u8>) -> Result<()> {
    // A per-tick budget also lets cancellation run when a child floods its pipes.
    for _ in 0..32 {
        let count = available(file).unwrap_or(0) as usize;
        if count == 0 {
            break;
        }
        let mut buffer = [0; 4096];
        let count = file.read(&mut buffer[..count.min(4096)])?;
        if count == 0 {
            break;
        }
        log.extend_from_slice(&buffer[..count]);
        if log.len() > LOG_LIMIT {
            log.drain(..log.len() - LOG_LIMIT);
        }
    }
    Ok(())
}

fn visible_log(log: &[u8]) -> String {
    let text = String::from_utf8_lossy(log);
    let mut visible = String::new();
    let mut escape = false;
    for ch in text.chars() {
        if ch == '\u{1b}' {
            escape = true;
            continue;
        }
        if escape {
            if ch.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        if ch == '\r' {
            visible.push('\n');
        } else if !ch.is_control() || ch == '\n' || ch == '\t' {
            visible.push(ch);
        }
    }
    let lines: Vec<_> = visible
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(6)..]
        .iter()
        .map(|line| line.chars().take(400).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "conduit CLI update {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn plan(&self, source: &str) -> Plan {
            let script = self.0.join("updater.ps1");
            std::fs::write(&script, source).unwrap();
            Plan {
                program: PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe"),
                args: vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-ExecutionPolicy".into(),
                    "Bypass".into(),
                    "-File".into(),
                    script.to_string_lossy().into_owned(),
                ],
                label: "fixture",
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cli_install_sources_match_selected_npm_package_and_reject_unrelated_binaries() {
        let _locale = crate::test_support::chinese();
        let fixture = Fixture::new();
        for provider in Provider::ALL {
            let directory = fixture.0.join("node_modules").join(package(provider));
            std::fs::create_dir_all(directory.join("bin")).unwrap();
            std::fs::write(
                directory.join("package.json"),
                serde_json::json!({"name": package(provider)}).to_string(),
            )
            .unwrap();
            let binary = directory.join("bin/engine.exe");
            std::fs::write(&binary, "fixture").unwrap();
            assert_eq!(npm_prefix(provider, &binary), Some(fixture.0.clone()));
            let prefix = fixture.0.join("中文 & %prefix% (global)");
            let node = fixture.0.join("node.exe");
            let script = fixture.0.join("npm-cli.js");
            let plan = Plan::npm(provider, &prefix, "1.2.3", node.clone(), script.clone());
            assert_eq!(plan.program, node);
            assert_eq!(plan.args[0], script.to_string_lossy());
            assert_eq!(&plan.args[1..4], ["install", "--global", "--prefix"]);
            assert_eq!(plan.args[4], prefix.to_string_lossy());
            assert_eq!(plan.args[5], format!("{}@1.2.3", package(provider)));
            let shim = fixture.0.join(format!("{}.cmd", provider.descriptor().id));
            std::fs::write(
                &shim,
                format!(
                    "@node \"%~dp0%node_modules\\{}\\bin\\cli.js\" %*",
                    package(provider).replace('/', "\\")
                ),
            )
            .unwrap();
            assert_eq!(npm_prefix(provider, &shim), Some(fixture.0.clone()));
            std::fs::write(&shim, "@echo unrelated").unwrap();
            assert_eq!(npm_prefix(provider, &shim), None);
            let unrelated = fixture.0.join(provider.descriptor().executable);
            assert_eq!(npm_prefix(provider, &unrelated), None);
        }
        let path = fixture.0.join("claude.exe");
        assert_eq!(
            Plan::detect(Provider::Claude, &path, "2.0.0").unwrap().args,
            ["update"]
        );
        let path = fixture.0.join("opencode.exe");
        assert_eq!(
            Plan::detect(Provider::OpenCode, &path, "1.2.0")
                .unwrap()
                .args,
            ["upgrade", "1.2.0"]
        );
        assert!(Plan::detect(Provider::Codex, &fixture.0.join("codex.exe"), "1.0.0").is_err());
        assert!(
            Plan::detect(
                Provider::Claude,
                &fixture.0.join("scoop/apps/claude/claude.exe"),
                "2.0.0"
            )
            .is_err()
        );
    }

    #[test]
    fn cli_install_streams_bounded_output_and_reports_stderr_exit_failures() {
        let _locale = crate::test_support::chinese();
        let fixture = Fixture::new();
        let plan = fixture.plan("[Console]::WriteLine(('x' * 30000)); [Console]::WriteLine('installing'); Start-Sleep -Milliseconds 600; [Console]::Error.WriteLine('permission denied'); exit 7");
        let mut progress = vec![];
        let error = run_command(
            &plan,
            &AtomicBool::new(false),
            Duration::from_secs(10),
            &mut |text| progress.push(text),
        )
        .unwrap_err();
        assert!(error.to_string().contains("退出码 7"));
        assert!(error.to_string().contains("permission denied"));
        assert!(!progress.is_empty());
        assert!(progress.iter().all(|text| text.len() < LOG_LIMIT));
        assert_eq!(visible_log(b"\x1b[31mfailed\x1b[0m\r\n"), "failed");
    }

    #[test]
    fn cli_install_cancellation_and_timeout_terminate_the_process_tree() {
        let _locale = crate::test_support::chinese();
        let fixture = Fixture::new();
        let plan = fixture.plan("[Console]::WriteLine('started'); Start-Sleep -Seconds 60");
        let cancelled = AtomicBool::new(false);
        let started = Instant::now();
        let error = run_command(&plan, &cancelled, Duration::from_secs(10), &mut |_| {
            cancelled.store(true, Ordering::Release)
        })
        .unwrap_err();
        assert!(error.to_string().contains("更新已停止"));
        assert!(started.elapsed() < Duration::from_secs(5));
        let error = run_command(
            &plan,
            &AtomicBool::new(false),
            Duration::from_millis(100),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("更新超时"));
    }

    #[test]
    fn cli_install_verifies_selected_executable_after_update_and_rejects_stale_versions() {
        let _locale = crate::test_support::chinese();
        let fixture = Fixture::new();
        let binary = fixture.0.join("codex.cmd");
        let mut plan = fixture.plan("param([string]$Target)\nSet-Content -LiteralPath $Target -Value '@echo codex-cli 1.2.0'");
        plan.args.push(binary.to_string_lossy().into_owned());
        let update = || UpdateInfo {
            latest_version: "1.2.0".into(),
            status: UpdateStatus::Available,
        };
        let result = perform(
            Provider::Codex,
            &binary,
            update(),
            &plan,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(result.version, "1.2.0");
        assert_eq!(
            result.update.unwrap().unwrap().status,
            UpdateStatus::Current
        );
        let plan = fixture.plan("exit 0");
        std::fs::write(&binary, "@echo codex-cli 1.1.0").unwrap();
        let error = perform(
            Provider::Codex,
            &binary,
            update(),
            &plan,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("尚未达到官方版本"));
    }
}
