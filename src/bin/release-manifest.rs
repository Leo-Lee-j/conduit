//! CI signing reads its seed at runtime. Explicit keygen writes a new seed to
//! the requested file; neither mode prints private keys or embeds them in apps.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use conduit::update::{
    REPOSITORY,
    manifest::{Manifest, hash_file, verify_manifest},
};
use ed25519_dalek::{Signer, SigningKey};
use std::{fs, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "keygen") {
        ensure!(
            args.len() == 2,
            "Usage: release-manifest keygen <private-key-output>"
        );
        let mut seed = [0_u8; 32];
        let status = unsafe {
            windows_sys::Win32::Security::Cryptography::BCryptGenRandom(
                std::ptr::null_mut(),
                seed.as_mut_ptr(),
                seed.len() as u32,
                windows_sys::Win32::Security::Cryptography::BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        ensure!(status >= 0, "Cannot generate signing key");
        let key = SigningKey::from_bytes(&seed);
        // Refuse to overwrite an existing private key. Print only the public key.
        use std::io::Write;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args[1])?;
        output.write_all(STANDARD.encode(key.to_bytes()).as_bytes())?;
        output.sync_all()?;
        println!("{}", STANDARD.encode(key.verifying_key().as_bytes()));
        return Ok(());
    }
    ensure!(
        args.len() == 2,
        "Usage: release-manifest <installer> <release-notes.md>"
    );
    let seed = STANDARD.decode(
        std::env::var("CONDUIT_UPDATE_PRIVATE_KEY")
            .context("Missing signing secret")?
            .trim(),
    )?;
    let bytes: [u8; 32] = seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Signing seed must contain 32 bytes"))?;
    let key = SigningKey::from_bytes(&bytes);
    let public = std::env::var("CONDUIT_UPDATE_PUBLIC_KEY").context("Missing public key")?;
    let installer = PathBuf::from(&args[0]);
    let version = env!("CARGO_PKG_VERSION");
    let filename = format!("Conduit-{version}-windows-x64-setup.exe");
    ensure!(
        installer
            .file_name()
            .is_some_and(|name| name == filename.as_str()),
        "Installer filename does not match Cargo version"
    );
    let (size, sha256) = hash_file(&installer)?;
    let manifest = Manifest {
        schema: 1,
        version: version.into(),
        target: "x86_64-pc-windows-msvc".into(),
        url: format!("https://github.com/{REPOSITORY}/releases/download/v{version}/{filename}"),
        filename,
        size,
        sha256,
        published_at: chrono::Utc::now().to_rfc3339(),
        notes: fs::read_to_string(&args[1])?,
        min_updater_version: 1,
    };
    let (raw, signature) = sign_manifest(&manifest, &key, &public)?;
    let output = installer.parent().context("Missing output directory")?;
    fs::write(output.join("update.json"), raw)?;
    fs::write(output.join("update.json.sig"), signature)?;
    println!("Signed update manifest for {version}");
    Ok(())
}

fn sign_manifest(manifest: &Manifest, key: &SigningKey, public: &str) -> Result<(Vec<u8>, String)> {
    ensure!(
        STANDARD.encode(key.verifying_key().as_bytes()) == public.trim(),
        "Signing key does not match public key"
    );
    manifest.validate()?;
    let raw = serde_json::to_vec_pretty(manifest)?;
    let signature = STANDARD.encode(key.sign(&raw).to_bytes());
    verify_manifest(&raw, &signature, public)?;
    Ok((raw, signature))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_tool_roundtrip_and_mismatched_key_rejection() {
        // Public, fixed test seed; no CI credentials or environment mutation.
        let key = SigningKey::from_bytes(&[7; 32]);
        let public = STANDARD.encode(key.verifying_key().as_bytes());
        let manifest = Manifest {
            schema: 1,
            version: "0.2.0".into(),
            target: "x86_64-pc-windows-msvc".into(),
            filename: "Conduit-0.2.0-windows-x64-setup.exe".into(),
            url: format!(
                "https://github.com/{REPOSITORY}/releases/download/v0.2.0/Conduit-0.2.0-windows-x64-setup.exe"
            ),
            size: 1,
            sha256: "0".repeat(64),
            published_at: "2026-11-02".into(),
            notes: "## 更新\n\n- 中文说明".into(),
            min_updater_version: 1,
        };
        let (raw, signature) = sign_manifest(&manifest, &key, &public).unwrap();
        let decoded = verify_manifest(&raw, &signature, &public).unwrap();
        assert_eq!(decoded.notes, manifest.notes);
        assert_eq!(decoded.sha256, manifest.sha256);
        let other = STANDARD.encode(SigningKey::from_bytes(&[8; 32]).verifying_key().as_bytes());
        assert!(sign_manifest(&manifest, &key, &other).is_err());
        let mut incompatible = manifest;
        incompatible.min_updater_version = 2;
        assert!(sign_manifest(&incompatible, &key, &public).is_err());
    }
}
