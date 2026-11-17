#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Keep a stable entry point for shortcuts and update helpers shipped before
// versioned process names. Never wait: only the versioned GUI stays running.
fn main() -> std::io::Result<()> {
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
