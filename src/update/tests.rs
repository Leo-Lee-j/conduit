use super::*;
#[test]
fn download_addresses_reject_untrusted_hosts_and_transport() {
    for address in [
        "http://github.com/x",
        "https://github.com.evil.test/x",
        "https://user@github.com/x",
        "https://github.com:444/x",
        "file:///x",
    ] {
        assert!(allowed_url(address).is_err(), "{address}");
    }
    assert!(allowed_url("https://release-assets.githubusercontent.com/test?token=test").is_ok());
}
#[test]
fn preferences_are_backward_compatible() {
    let p: Preferences = serde_json::from_str("{}").unwrap();
    assert!(p.auto_check && p.auto_download && !p.auto_install);
}

#[test]
fn metadata_retries_transient_failures_with_bounded_backoff() {
    let mut attempts = 0;
    let mut waits = Vec::new();
    let value = retry_metadata(
        || {
            attempts += 1;
            match attempts {
                1 => Err(ureq::Error::ConnectionFailed.into()),
                2 => Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
                _ => Ok(b"complete response".to_vec()),
            }
        },
        |delay| waits.push(delay),
    )
    .unwrap();
    assert_eq!(value, b"complete response");
    assert_eq!(attempts, 3);
    assert_eq!(waits, [Duration::from_millis(500), Duration::from_secs(1)]);
}

#[test]
fn metadata_stops_after_three_attempts_and_keeps_the_failure() {
    let mut attempts = 0;
    let error = retry_metadata::<()>(
        || {
            attempts += 1;
            Err(anyhow::Error::new(ureq::Error::StatusCode(503)).context("service unavailable"))
        },
        |_| {},
    )
    .unwrap_err();
    assert_eq!(attempts, 3);
    assert!(matches!(
        error.downcast_ref::<ureq::Error>(),
        Some(ureq::Error::StatusCode(503))
    ));
}

#[test]
fn permanent_and_security_failures_are_not_retried() {
    for error in [
        anyhow::Error::new(ureq::Error::StatusCode(404)),
        anyhow::Error::new(ureq::Error::StatusCode(403)),
        anyhow::Error::new(ureq::Error::Tls("certificate verification failed")),
        anyhow::anyhow!("metadata exceeds size limit"),
        allowed_url("https://untrusted.example/update.json").unwrap_err(),
    ] {
        let mut error = Some(error);
        assert!(
            retry_metadata::<()>(
                || Err(error.take().expect("must not retry permanent errors")),
                |_| panic!("must not wait on permanent errors"),
            )
            .is_err()
        );
    }
    for status in [408, 429, 500, 502, 503, 504] {
        assert!(transient(&anyhow::Error::new(ureq::Error::StatusCode(
            status
        ))));
    }
}

fn signed_release(version: &str) -> (Release, String) {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use ed25519_dalek::{Signer, SigningKey};
    let manifest = Manifest {
        schema: 1,
        version: version.into(),
        target: "x86_64-pc-windows-msvc".into(),
        filename: format!("Conduit-{version}-windows-x64-setup.exe"),
        url: format!(
            "https://github.com/{REPOSITORY}/releases/download/v{version}/Conduit-{version}-windows-x64-setup.exe"
        ),
        size: 123,
        sha256: "a".repeat(64),
        published_at: "2026-11-08T09:30:00Z".into(),
        notes: "Test release".into(),
        min_updater_version: 1,
    };
    let raw = serde_json::to_vec(&manifest).unwrap();
    let signer = SigningKey::from_bytes(&[7; 32]);
    let signature = STANDARD.encode(signer.sign(&raw).to_bytes());
    (
        Release {
            manifest,
            raw,
            signature,
        },
        STANDARD.encode(signer.verifying_key().as_bytes()),
    )
}

fn api_response(release: &Release) -> serde_json::Value {
    let version = &release.manifest.version;
    serde_json::json!({
        "tag_name": format!("v{version}"), "draft": false, "prerelease": false,
        "assets": [
            { "name": "update.json", "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/update.json") },
            { "name": "update.json.sig", "browser_download_url": format!("https://github.com/{REPOSITORY}/releases/download/v{version}/update.json.sig") },
            { "name": release.manifest.filename, "browser_download_url": release.manifest.url },
        ],
    })
}

fn check_responses(
    responses: Vec<(String, Result<Vec<u8>>)>,
    key: &str,
    current: &str,
) -> Result<Option<Release>> {
    let mut responses = responses.into_iter();
    let result = check_release_with(
        |url| {
            let (expected, response) = responses.next().expect("unexpected fetch or fallback");
            assert_eq!(url, expected);
            response
        },
        key,
        current,
    );
    assert!(
        responses.next().is_none(),
        "not all expected requests were sent"
    );
    result
}

fn api_url() -> String {
    format!("https://api.github.com/repos/{REPOSITORY}/releases/latest")
}

#[test]
fn api_outage_uses_official_signed_manifest_and_version_pinned_signature() {
    let (release, key) = signed_release("0.3.0");
    for error in [
        ureq::Error::StatusCode(403),
        ureq::Error::StatusCode(429),
        ureq::Error::StatusCode(503),
        ureq::Error::ConnectionFailed,
    ] {
        let found = check_responses(
            vec![
                (api_url(), Err(error.into())),
                (
                    format!("https://github.com/{REPOSITORY}/releases/latest/download/update.json"),
                    Ok(release.raw.clone()),
                ),
                (
                    format!(
                        "https://github.com/{REPOSITORY}/releases/download/v0.3.0/update.json.sig"
                    ),
                    Ok(release.signature.as_bytes().to_vec()),
                ),
            ],
            &key,
            "0.2.4",
        )
        .unwrap()
        .unwrap();
        assert_eq!(found.manifest.version, "0.3.0");
        assert_eq!(found.raw, release.raw);
    }
}

#[test]
fn fallback_rejects_bad_signatures_unsafe_sources_and_non_upgrades() {
    let (release, key) = signed_release("0.3.0");
    let latest = format!("https://github.com/{REPOSITORY}/releases/latest/download/update.json");
    let signature =
        format!("https://github.com/{REPOSITORY}/releases/download/v0.3.0/update.json.sig");
    let mut tampered = release.raw.clone();
    tampered.push(b' '); // Still valid JSON, but the signature no longer matches.
    let error = check_responses(
        vec![
            (api_url(), Err(ureq::Error::StatusCode(403).into())),
            (latest.clone(), Ok(tampered)),
            (signature.clone(), Ok(release.signature.as_bytes().to_vec())),
        ],
        &key,
        "0.2.4",
    )
    .unwrap_err();
    assert!(format!("{error:#}").contains("签名验证失败"));

    let mut unsafe_manifest = release.manifest.clone();
    unsafe_manifest.url = "https://untrusted.example/installer.exe".into();
    assert!(
        check_responses(
            vec![
                (api_url(), Err(ureq::Error::StatusCode(403).into())),
                (
                    latest.clone(),
                    Ok(serde_json::to_vec(&unsafe_manifest).unwrap())
                ),
            ],
            &key,
            "0.2.4"
        )
        .is_err()
    );

    for current in ["0.3.0", "0.4.0"] {
        assert!(
            check_responses(
                vec![
                    (api_url(), Err(ureq::Error::StatusCode(403).into())),
                    (latest.clone(), Ok(release.raw.clone())),
                    (signature.clone(), Ok(release.signature.as_bytes().to_vec())),
                ],
                &key,
                current
            )
            .unwrap()
            .is_none()
        );
    }
}

#[test]
fn successful_api_check_validates_manifest_and_assets_without_fallback() {
    let (release, key) = signed_release("0.3.0");
    let api = api_response(&release);
    let found = check_responses(
        vec![
            (api_url(), Ok(serde_json::to_vec(&api).unwrap())),
            (
                api["assets"][0]["browser_download_url"]
                    .as_str()
                    .unwrap()
                    .into(),
                Ok(release.raw.clone()),
            ),
            (
                api["assets"][1]["browser_download_url"]
                    .as_str()
                    .unwrap()
                    .into(),
                Ok(release.signature.as_bytes().to_vec()),
            ),
        ],
        &key,
        "0.2.4",
    )
    .unwrap()
    .unwrap();
    assert_eq!(found.manifest.version, "0.3.0");
}

#[test]
fn invalid_api_releases_and_signatures_do_not_trigger_fallback() {
    let (release, key) = signed_release("0.3.0");
    for fault in ["tag", "asset", "signature"] {
        let mut api = api_response(&release);
        let mut raw = release.raw.clone();
        match fault {
            "tag" => api["tag_name"] = "v0.4.0".into(),
            "asset" => api["assets"]
                .as_array_mut()
                .unwrap()
                .pop()
                .map(|_| ())
                .unwrap(),
            "signature" => raw.push(b' '),
            _ => unreachable!(),
        }
        assert!(
            check_responses(
                vec![
                    (api_url(), Ok(serde_json::to_vec(&api).unwrap())),
                    (
                        api["assets"][0]["browser_download_url"]
                            .as_str()
                            .unwrap()
                            .into(),
                        Ok(raw)
                    ),
                    (
                        api["assets"][1]["browser_download_url"]
                            .as_str()
                            .unwrap()
                            .into(),
                        Ok(release.signature.as_bytes().to_vec())
                    ),
                ],
                &key,
                "0.2.4"
            )
            .is_err(),
            "{fault}"
        );
    }
    assert!(check_responses(vec![(api_url(), Ok(b"not JSON".to_vec()))], &key, "0.2.4").is_err());
    assert!(
        check_responses(
            vec![(
                api_url(),
                Err(ureq::Error::Tls("invalid certificate").into())
            )],
            &key,
            "0.2.4"
        )
        .is_err()
    );
    assert!(check_responses(Vec::new(), "", "0.2.4").is_err());
}

#[test]
fn absent_releases_remain_no_update_but_failed_fallbacks_remain_errors() {
    let (_, key) = signed_release("0.3.0");
    assert!(
        check_responses(
            vec![(api_url(), Err(ureq::Error::StatusCode(404).into()))],
            &key,
            "0.2.4"
        )
        .unwrap()
        .is_none()
    );
    let error = check_responses(
        vec![
            (api_url(), Err(ureq::Error::StatusCode(403).into())),
            (
                format!("https://github.com/{REPOSITORY}/releases/latest/download/update.json"),
                Err(ureq::Error::StatusCode(503).into()),
            ),
        ],
        &key,
        "0.2.4",
    )
    .unwrap_err();
    let error = format!("{error:#}");
    assert!(error.contains("403") && error.contains("503"), "{error}");
}
