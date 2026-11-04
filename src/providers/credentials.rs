//! Protect manually entered service credentials for the current Windows user.
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub fn protect(secret: &str) -> Result<String> {
    ensure!(
        !secret.trim().is_empty() && secret.len() <= 16384,
        "请输入有效的 API Key（最多 16 KiB）"
    );
    Ok(STANDARD.encode(transform(secret.as_bytes(), true)?))
}

pub fn unprotect(ciphertext: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(ciphertext)
        .map_err(|_| anyhow::anyhow!("保存的凭据无效，请重新输入 API Key"))?;
    String::from_utf8(transform(&bytes, false)?)
        .map_err(|_| anyhow::anyhow!("凭据无法读取，请重新输入 API Key"))
}

#[cfg(windows)]
fn transform(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    ensure!(bytes.len() <= 32768, "凭据数据过长");
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI allocates output with LocalAlloc; release it after copying. No
    // machine-wide flag: only the current Windows account can decrypt it.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    ensure!(
        ok != 0,
        "Windows 凭据保护失败；请在原 Windows 账户下使用，或重新输入 API Key"
    );
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        for i in 0..output.cbData as usize {
            std::ptr::write_volatile(output.pbData.add(i), 0);
        }
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(not(windows))]
fn transform(_: &[u8], _: bool) -> Result<Vec<u8>> {
    anyhow::bail!("当前平台请使用环境变量保存凭据")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn protects_and_roundtrips_without_plaintext() {
        let secret = "conduit-test-key-不是真实凭证";
        let encrypted = protect(secret).unwrap();
        assert!(!encrypted.contains(secret));
        assert_eq!(unprotect(&encrypted).unwrap(), secret);
        assert!(unprotect("broken").is_err());
        assert!(protect("").is_err());
    }
}
