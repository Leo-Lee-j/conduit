//! Open HTML in the default HTTPS browser, independent of its file association.
use anyhow::{Result, ensure};
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::Path};
use windows_sys::Win32::UI::Shell::{
    ASSOCF_IS_PROTOCOL, ASSOCF_VERIFY, ASSOCSTR_EXECUTABLE, AssocQueryStringW,
};

pub(super) fn is_html(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("html") || extension.eq_ignore_ascii_case("htm")
        })
}

fn default_browser() -> Result<OsString> {
    let protocol: Vec<u16> = "https\0".encode_utf16().collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let mut len = 0;
    // The first call returns S_FALSE and the required buffer length. Query the
    // protocol association so an HTML editor cannot intercept this action.
    let query = |output, len: &mut u32| unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_VERIFY,
            ASSOCSTR_EXECUTABLE,
            protocol.as_ptr(),
            verb.as_ptr(),
            output,
            len,
        )
    };
    let result = query(std::ptr::null_mut(), &mut len);
    ensure!(
        result >= 0 && len > 1,
        "HTTPS association: 0x{:08X}",
        result as u32
    );
    let mut executable = vec![0; len as usize];
    let result = query(executable.as_mut_ptr(), &mut len);
    ensure!(result == 0, "HTTPS association: 0x{:08X}", result as u32);
    let end = executable
        .iter()
        .position(|ch| *ch == 0)
        .unwrap_or(executable.len());
    ensure!(end > 0, "Empty HTTPS executable");
    Ok(OsString::from_wide(&executable[..end]))
}

pub(super) fn open(path: &Path) -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let path = crate::child::normalized_dir(&std::fs::canonicalize(path)?);
    ensure!(
        path.is_file(),
        "{}",
        conduit::i18n::text_static("files.unable_to_preview_this_file")
    );
    let url = url::Url::from_file_path(&path)
        .map_err(|_| anyhow::anyhow!(conduit::i18n::text_static("artifact.invalid")))?;
    Command::new(default_browser()?)
        .arg(url.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    Ok(())
}
