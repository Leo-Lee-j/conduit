#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Keep a stable entry point for shortcuts and update helpers shipped before
// versioned process names. Never wait: only the versioned GUI stays running.
fn main() {
    // A missing versioned executable after an interrupted update is the most
    // common reason Conduit fails to open; without a console it must be visible.
    if let Err(error) = launch() {
        conduit::diagnostics::record_crash("launcher", &error.to_string());
        let text: Vec<u16> = format!(
            "{error}\n\n{}",
            conduit::diagnostics::crash_log_path().display()
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let title: Vec<u16> = "Conduit".encode_utf16().chain(Some(0)).collect();
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

fn launch() -> std::io::Result<()> {
    let executable = std::env::current_exe()?;
    let directory = executable.parent().ok_or_else(|| {
        std::io::Error::other("Cannot determine the Conduit installation directory")
    })?;
    std::process::Command::new(directory.join(concat!(
        "conduit-",
        env!("CARGO_PKG_VERSION"),
        ".exe"
    )))
    .args(std::env::args_os().skip(1))
    .spawn()?;
    Ok(())
}
