#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let result = std::env::args_os()
        .nth(1)
        .ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text(
                "bin.conduit-updater.update_handoff_file_missing"
            ))
        })
        .and_then(|path| conduit::update::install::run(std::path::Path::new(&path)));
    if let Err(error) = result {
        let text: Vec<u16> = conduit::i18n::format(
            "bin.conduit-updater.conduit_update_incomplete",
            &[("error", format!("{:#}", error))],
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let title: Vec<u16> = conduit::i18n::text_static("bin.conduit-updater.conduit_update")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
