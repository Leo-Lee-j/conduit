use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub const MAX_PACKAGE_SIZE: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub target: String,
    pub filename: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub published_at: String,
    pub notes: String,
    pub min_updater_version: u32,
}

impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.min_updater_version <= 1,
            "此更新需要更新版本的安装器"
        );
        let version = semver::Version::parse(&self.version)?;
        ensure!(
            version.pre.is_empty() && version.build.is_empty(),
            "当前仅支持稳定版本"
        );
        ensure!(self.target == "x86_64-pc-windows-msvc", "更新包平台不匹配");
        ensure!(
            self.filename == format!("Conduit-{}-windows-x64-setup.exe", self.version),
            "更新文件名不匹配"
        );
        ensure!(
            self.url
                == format!(
                    "https://github.com/{}/releases/download/v{}/{}",
                    super::REPOSITORY,
                    self.version,
                    self.filename
                ),
            "更新来源不匹配"
        );
        ensure!(
            self.size > 0 && self.size <= MAX_PACKAGE_SIZE,
            "更新大小无效"
        );
        ensure!(
            self.sha256.len() == 64 && self.sha256.bytes().all(|c| c.is_ascii_hexdigit()),
            "SHA-256 无效"
        );
        ensure!(self.notes.len() <= 256 * 1024, "更新说明过长");
        Ok(())
    }
}

pub fn verify_manifest(raw: &[u8], signature: &str, public_key: &str) -> Result<Manifest> {
    ensure!(raw.len() <= 1024 * 1024, "更新清单过大");
    let key: [u8; 32] = STANDARD
        .decode(public_key.trim())
        .context("更新公钥无效")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("更新公钥长度无效"))?;
    let signature = Signature::from_slice(
        &STANDARD
            .decode(signature.trim())
            .context("更新签名编码无效")?,
    )?;
    VerifyingKey::from_bytes(&key)?
        .verify_strict(raw, &signature)
        .context("更新签名验证失败")?;
    let manifest: Manifest = serde_json::from_slice(raw)?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn hash_file(path: &Path) -> Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut size = 0;
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        size += len as u64;
        ensure!(size <= MAX_PACKAGE_SIZE, "安装包超出大小限制");
        digest.update(&buffer[..len]);
    }
    Ok((size, format!("{:x}", digest.finalize())))
}

pub fn verify_package(path: &Path, manifest: &Manifest) -> Result<()> {
    ensure!(path.metadata()?.len() == manifest.size, "安装包大小不匹配");
    let (_, digest) = hash_file(path)?;
    ensure!(
        digest.eq_ignore_ascii_case(&manifest.sha256),
        "安装包 SHA-256 不匹配"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    pub fn fixture() -> Manifest {
        Manifest {
            schema: 1,
            version: "0.2.0".into(),
            target: "x86_64-pc-windows-msvc".into(),
            filename: "Conduit-0.2.0-windows-x64-setup.exe".into(),
            url: format!(
                "https://github.com/{}/releases/download/v0.2.0/Conduit-0.2.0-windows-x64-setup.exe",
                super::super::REPOSITORY
            ),
            size: 1,
            sha256: "0".repeat(64),
            published_at: "2026-11-02".into(),
            notes: "修复问题".into(),
            min_updater_version: 1,
        }
    }

    #[test]
    fn authenticity_covers_every_byte_and_key() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let key = STANDARD.encode(signer.verifying_key().as_bytes());
        let mut bytes = serde_json::to_vec(&fixture()).unwrap();
        let sig = STANDARD.encode(signer.sign(&bytes).to_bytes());
        assert!(verify_manifest(&bytes, &sig, &key).is_ok());
        bytes.push(b' ');
        assert!(verify_manifest(&bytes, &sig, &key).is_err());
        let wrong_key =
            STANDARD.encode(SigningKey::from_bytes(&[8; 32]).verifying_key().as_bytes());
        assert!(verify_manifest(&bytes[..bytes.len() - 1], &sig, &wrong_key).is_err());
    }

    #[test]
    fn incompatible_and_unsafe_manifests_fail_closed() {
        for mutate in [
            |m: &mut Manifest| m.filename = "../evil.exe".into(),
            |m: &mut Manifest| m.url = "https://example.com/a.exe".into(),
            |m: &mut Manifest| m.target = "aarch64-pc-windows-msvc".into(),
            |m: &mut Manifest| m.min_updater_version = 2,
            |m: &mut Manifest| m.size = MAX_PACKAGE_SIZE + 1,
            |m: &mut Manifest| m.version = "0.2.0-beta.1".into(),
        ] {
            let mut manifest = fixture();
            mutate(&mut manifest);
            assert!(manifest.validate().is_err());
        }
    }

    #[test]
    fn package_verification_rejects_truncated_and_same_size_corruption() {
        let path =
            std::env::temp_dir().join(format!("conduit-package-test-{}.exe", std::process::id()));
        std::fs::write(&path, b"package").unwrap();
        let (size, sha256) = hash_file(&path).unwrap();
        let mut manifest = fixture();
        manifest.size = size;
        manifest.sha256 = sha256;
        verify_package(&path, &manifest).unwrap();
        std::fs::write(&path, b"packagE").unwrap();
        assert!(verify_package(&path, &manifest).is_err());
        std::fs::write(&path, b"pack").unwrap();
        assert!(verify_package(&path, &manifest).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
