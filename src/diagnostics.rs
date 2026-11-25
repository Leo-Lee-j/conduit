//! Bounded, content-free diagnostic logs shared by the app and its helpers.
//! Windows release builds have no console, so these files are the only trace
//! of failures that happen before the UI exists or outside the main process.
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

/// Crash records are rare; a small cap keeps the file readable and bounded.
pub const CRASH_LOG_LIMIT: u64 = 64 * 1024;
const LINE_LIMIT: usize = 2048;
const PANIC_MESSAGE_LIMIT: usize = 512;

/// Directory shared with persisted state. `CONDUIT_DATA_DIR` overrides it;
/// debug builds stay separate from the installed app.
pub fn data_dir() -> PathBuf {
    std::env::var_os("CONDUIT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_data_dir)
}

pub fn default_data_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_else(|| ".".into())).join(
        if cfg!(debug_assertions) {
            "Conduit-dev"
        } else {
            "Conduit"
        },
    )
}

pub fn crash_log_path() -> PathBuf {
    data_dir().join("crash.log")
}

/// Appends one timestamped line, restarting the file rather than exceeding
/// `limit`. Control characters become spaces so a message cannot forge
/// extra entries.
pub fn append_bounded(path: &Path, message: &str, limit: u64) -> io::Result<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let message: String = message
        .chars()
        .take(LINE_LIMIT)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let line = format!("{now} {message}\n");
    if line.len() as u64 > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "diagnostic line exceeds the log limit",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let truncate = std::fs::metadata(path)
        .is_ok_and(|meta| meta.len().saturating_add(line.len() as u64) > limit);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)?;
    file.write_all(line.as_bytes())
}

/// Best effort: a failure to record must never mask the original failure.
pub fn record_crash(context: &str, message: &str) {
    let _ = append_bounded(
        &crash_log_path(),
        &format!("v{} {context}: {message}", env!("CARGO_PKG_VERSION")),
        CRASH_LOG_LIMIT,
    );
}

/// Records panics without payload dumps or backtraces, then defers to the
/// previous hook so debug output and unwinding behave as before.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("non-string panic payload");
        let message: String = message.chars().take(PANIC_MESSAGE_LIMIT).collect();
        let location = info
            .location()
            .map(|location| format!("{}:{}", location.file(), location.line()))
            .unwrap_or_else(|| "unknown".to_owned());
        let thread = std::thread::current();
        record_crash(
            "panic",
            &format!(
                "thread={} at {location}: {message}",
                thread.name().unwrap_or("unnamed")
            ),
        );
        previous(info);
    }));
}
