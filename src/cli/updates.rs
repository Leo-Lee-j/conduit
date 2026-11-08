//! Read-only checks against each engine's official stable GitHub release.
use super::*;
use semver::Version;

mod install;
pub(super) use install::run as install;

const MAX_RELEASE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    Available,
    Current,
    Ahead,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub latest_version: String,
    pub status: UpdateStatus,
}

impl UpdateInfo {
    pub fn summary(&self) -> String {
        let status = match self.status {
            UpdateStatus::Available => conduit::i18n::text_static("cli.updates.update_available"),
            UpdateStatus::Current => conduit::i18n::text_static("cli.updates.up_to_date"),
            UpdateStatus::Ahead => conduit::i18n::text_static(
                "cli.updates.installed_version_is_newer_than_the_official",
            ),
        };
        conduit::i18n::format(
            "cli.updates.latest_official_version",
            &[
                ("arg0", self.latest_version.to_string()),
                ("status", status.to_string()),
            ],
        )
    }
}

pub fn release_url(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "https://api.github.com/repos/anthropics/claude-code/releases/latest",
        Provider::Codex => "https://api.github.com/repos/openai/codex/releases/latest",
        Provider::OpenCode => "https://api.github.com/repos/anomalyco/opencode/releases/latest",
    }
}

pub fn guide_url(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "https://code.claude.com/docs/en/setup#update-claude-code",
        Provider::Codex => "https://developers.openai.com/codex/cli",
        Provider::OpenCode => "https://opencode.ai/docs/cli/#upgrade",
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn compare_release(provider: Provider, installed: &str, bytes: &[u8]) -> Result<UpdateInfo> {
    let release: Release = serde_json::from_slice(bytes).context(conduit::i18n::text_static(
        "cli.updates.unable_to_parse_official_version_information",
    ))?;
    anyhow::ensure!(
        !release.draft && !release.prerelease,
        conduit::i18n::text("cli.updates.update_service_returned_a_non_stable_version")
    );
    let tag = release.tag_name.trim();
    let tag = if provider == Provider::Codex {
        tag.strip_prefix("rust-v").unwrap_or(tag)
    } else {
        tag
    };
    let latest = Version::parse(tag.strip_prefix('v').unwrap_or(tag)).context(
        conduit::i18n::text_static("cli.updates.unrecognized_official_version_format"),
    )?;
    anyhow::ensure!(
        latest.pre.is_empty(),
        conduit::i18n::text("cli.updates.update_service_returned_a_prerelease_version")
    );
    let current = Version::parse(installed).context(conduit::i18n::text_static(
        "cli.updates.unable_to_compare_the_current_cli_version",
    ))?;
    let status = match current.cmp_precedence(&latest) {
        std::cmp::Ordering::Less => UpdateStatus::Available,
        std::cmp::Ordering::Equal => UpdateStatus::Current,
        std::cmp::Ordering::Greater => UpdateStatus::Ahead,
    };
    Ok(UpdateInfo {
        latest_version: latest.to_string(),
        status,
    })
}

pub(super) fn check(
    provider: Provider,
    installed: &str,
    cancelled: &AtomicBool,
) -> Result<UpdateInfo> {
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        conduit::i18n::text("settings.cli.detection_cancelled")
    );
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(0)
        .timeout_connect(Some(Duration::from_secs(5)))
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = agent
        .get(release_url(provider))
        .header("User-Agent", concat!("Conduit/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| match error {
            ureq::Error::StatusCode(403 | 429) => {
                anyhow::anyhow!(conduit::i18n::text(
                    "cli.updates.official_update_service_rate_limit_reached_try"
                ))
            }
            other => anyhow::anyhow!(conduit::i18n::format(
                "cli.updates.unable_to_reach_the_official_update_service",
                &[("other", format!("{}", other))]
            )),
        })?;
    anyhow::ensure!(
        response.status().is_success(),
        conduit::i18n::text("cli.updates.official_update_service_returned_an_unexpected_response")
    );
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        conduit::i18n::text("settings.cli.detection_cancelled")
    );
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(MAX_RELEASE_BYTES + 1)
        .read_to_end(&mut bytes)
        .context(conduit::i18n::text_static(
            "cli.updates.failed_to_read_official_version_information",
        ))?;
    anyhow::ensure!(
        !cancelled.load(Ordering::Acquire),
        conduit::i18n::text("settings.cli.detection_cancelled")
    );
    anyhow::ensure!(
        bytes.len() as u64 <= MAX_RELEASE_BYTES,
        conduit::i18n::text("cli.updates.official_version_information_exceeds_the_size_limit")
    );
    compare_release(provider, installed, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "tag_name": tag, "draft": false, "prerelease": false,
        }))
        .unwrap()
    }

    #[test]
    fn official_cli_updates_compare_numeric_versions_and_all_release_tag_formats() {
        let _locale = crate::test_support::chinese();
        for (provider, installed, tag) in [
            (Provider::Claude, "2.1.9", "v2.1.10"),
            (Provider::Codex, "0.9.0", "rust-v0.10.0"),
            (Provider::OpenCode, "1.9.0", "v1.10.0"),
        ] {
            let update = compare_release(provider, installed, &release(tag)).unwrap();
            assert_eq!(update.status, UpdateStatus::Available);
            assert!(update.summary().contains("发现可用更新"));
            assert!(release_url(provider).starts_with("https://api.github.com/repos/"));
        }
        for (installed, status) in [
            ("0.10.0", UpdateStatus::Current),
            ("0.10.0+local.1", UpdateStatus::Current),
            ("0.11.0", UpdateStatus::Ahead),
            ("0.11.0-alpha.1", UpdateStatus::Ahead),
            ("0.10.0-alpha.1", UpdateStatus::Available),
        ] {
            assert_eq!(
                compare_release(Provider::Codex, installed, &release("rust-v0.10.0"))
                    .unwrap()
                    .status,
                status
            );
        }
    }

    #[test]
    fn official_cli_updates_reject_invalid_or_unstable_metadata_and_support_cancellation() {
        let _locale = crate::test_support::chinese();
        for bytes in [
            br#"{}"#.as_slice(),
            br#"not json"#,
            br#"{"tag_name":"v2.1.0","draft":true,"prerelease":false}"#,
            br#"{"tag_name":"v2.1.0","draft":false,"prerelease":true}"#,
            br#"{"tag_name":"v2.1.0-beta.1","draft":false,"prerelease":false}"#,
            br#"{"tag_name":"garbage","draft":false,"prerelease":false}"#,
        ] {
            assert!(compare_release(Provider::Claude, "2.0.0", bytes).is_err());
        }
        assert!(compare_release(Provider::Claude, "unrecognized", &release("v2.1.0")).is_err());
        let error = check(Provider::Claude, "2.0.0", &AtomicBool::new(true)).unwrap_err();
        assert!(error.to_string().contains("已取消"));
    }

    #[test]
    #[ignore = "requires installed CLIs and network; reads --version and official release metadata only"]
    fn installed_cli_updates_report_official_versions() {
        let _locale = crate::test_support::chinese();
        let mut checked = 0;
        for provider in Provider::ALL {
            let Ok(resolved) = resolve(provider, &CliSettings::default()) else {
                continue;
            };
            let probe = VersionProbe::start_update_check(provider, resolved.path);
            let info = probe
                .events
                .recv_timeout(Duration::from_secs(30))
                .unwrap()
                .unwrap();
            assert!(
                info.update.as_ref().unwrap().is_ok(),
                "{}: {}",
                provider.label(),
                info.diagnostic()
            );
            println!("{}: {}", provider.label(), info.diagnostic());
            checked += 1;
        }
        assert!(checked > 0, "no installed CLI was checked");
    }
}
