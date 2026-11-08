//! Protect manually entered service credentials for the current Windows user.
use anyhow::{Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub fn protect(secret: &str) -> Result<String> {
    ensure!(
        !secret.trim().is_empty() && secret.len() <= 16384,
        conduit::i18n::text("providers.credentials.enter_a_valid_api_key_up_to")
    );
    Ok(STANDARD.encode(transform(secret.as_bytes(), true)?))
}

pub fn unprotect(ciphertext: &str) -> Result<String> {
    let bytes = STANDARD.decode(ciphertext).map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text(
            "providers.credentials.saved_credentials_are_invalid_re_enter_the"
        ))
    })?;
    String::from_utf8(transform(&bytes, false)?).map_err(|_| {
        anyhow::anyhow!(conduit::i18n::text(
            "providers.credentials.unable_to_read_credentials_re_enter_the"
        ))
    })
}

#[cfg(windows)]
fn transform(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    ensure!(
        bytes.len() <= 32768,
        conduit::i18n::text("providers.credentials.credential_data_is_too_long")
    );
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
        conduit::i18n::text(
            "providers.credentials.windows_credential_protection_failed_use_the_original"
        )
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
    anyhow::bail!(conduit::i18n::text(
        "providers.credentials.use_environment_variables_for_credentials_on_this"
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn protects_and_roundtrips_without_plaintext() {
        let _locale = crate::test_support::chinese();
        let secret = "conduit-test-key-不是真实凭证";
        let encrypted = protect(secret).unwrap();
        assert!(!encrypted.contains(secret));
        assert_eq!(unprotect(&encrypted).unwrap(), secret);
        assert!(unprotect("broken").is_err());
        assert!(protect("").is_err());
    }
}
