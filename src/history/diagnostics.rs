//! Bounded, content-free summaries for native history imports.
use std::{
    io::{self, Write},
    path::Path,
};

const SCAN_LOG_LIMIT: u64 = 64 * 1024;
const SUMMARY_LIMIT: usize = 2048;

pub(crate) fn record_scan_summary(summary: &str) -> io::Result<()> {
    let path = crate::store::Store::path().with_file_name("history-scan.log");
    append_summary(&path, summary)
}

fn append_summary(path: &Path, summary: &str) -> io::Result<()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let summary: String = summary
        .chars()
        .take(SUMMARY_LIMIT)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let line = format!("{now} {summary}\n");
    if line.len() as u64 > SCAN_LOG_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "history scan summary exceeds the log limit",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let truncate = std::fs::metadata(path)
        .is_ok_and(|meta| meta.len().saturating_add(line.len() as u64) > SCAN_LOG_LIMIT);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!truncate)
        .truncate(truncate)
        .open(path)?;
    file.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_log_is_bounded_and_sanitizes_control_characters() {
        let path =
            std::env::temp_dir().join(format!("conduit-history-scan-{}.log", std::process::id()));
        std::fs::write(&path, vec![b'x'; SCAN_LOG_LIMIT as usize]).unwrap();
        append_summary(&path, "history_import total_ms=12\ninjected").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.len() < 256);
        assert!(contents.contains("history_import total_ms=12 injected"));
        assert_eq!(contents.lines().count(), 1);
        std::fs::remove_file(path).unwrap();
    }
}
