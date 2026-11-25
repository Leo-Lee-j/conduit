//! Bounded, content-free summaries for native history imports.
use std::{io, path::Path};

const SCAN_LOG_LIMIT: u64 = 64 * 1024;

pub(crate) fn record_scan_summary(summary: &str) -> io::Result<()> {
    let path = crate::store::Store::path().with_file_name("history-scan.log");
    append_summary(&path, summary)
}

fn append_summary(path: &Path, summary: &str) -> io::Result<()> {
    conduit::diagnostics::append_bounded(path, summary, SCAN_LOG_LIMIT)
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
