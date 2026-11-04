//! Incremental, bounded native-history discovery. Failed or changing files
//! never produce a partial transcript that could replace a saved conversation.
use super::*;
use std::{
    collections::VecDeque,
    fs::File,
    io::{BufRead, BufReader, Read},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
const MAX_FILE: u64 = 32 * 1024 * 1024;
const MAX_SCAN: u64 = 512 * 1024 * 1024;
const MAX_LINE: usize = 8 * 1024 * 1024;
const CACHE_BYTES: usize = 32 * 1024 * 1024;
type Parsed = (Vec<String>, ImportedSession);
struct Cached {
    path: PathBuf,
    stamp: crate::workspace_files::Stamp,
    parsed: Parsed,
    bytes: usize,
}
static CACHE: LazyLock<Mutex<VecDeque<Cached>>> = LazyLock::new(|| Mutex::new(VecDeque::new()));
pub(crate) struct Batch {
    pub sessions: Vec<Vec<ImportedSession>>,
    pub warnings: Vec<String>,
}

struct Lines<'a> {
    reader: BufReader<File>,
    read: u64,
    cancel: &'a AtomicBool,
    error: Option<String>,
}
impl Iterator for Lines<'_> {
    type Item = String;
    fn next(&mut self) -> Option<String> {
        if self.error.is_some() {
            return None;
        }
        if self.cancel.load(Ordering::Acquire) {
            self.error = Some("历史读取已取消".into());
            return None;
        }
        let mut bytes = Vec::new();
        match self
            .reader
            .by_ref()
            .take((MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut bytes)
        {
            Ok(0) => None,
            Ok(n) => {
                self.read += n as u64;
                if n > MAX_LINE || self.read > MAX_FILE {
                    self.error = Some("历史文件或单行超过读取上限".into());
                    return None;
                }
                match String::from_utf8(bytes) {
                    Ok(s) => Some(s),
                    Err(_) => {
                        self.error = Some("历史文件不是 UTF-8".into());
                        None
                    }
                }
            }
            Err(e) => {
                self.error = Some(format!("无法读取历史：{e}"));
                None
            }
        }
    }
}

pub(crate) fn run(home: Option<&Path>, workspaces: &[PathBuf], cancel: &AtomicBool) -> Batch {
    let mut batch = Batch {
        sessions: vec![vec![]; workspaces.len()],
        warnings: vec![],
    };
    let Some(home) = home else {
        return batch;
    };
    if workspaces.is_empty() {
        return batch;
    }
    let mut remaining = MAX_SCAN;
    for provider in [Provider::Claude, Provider::Codex] {
        let root = home.join(match provider {
            Provider::Claude => ".claude/projects",
            _ => ".codex/sessions",
        });
        let mut stack = vec![root];
        while let Some(directory) = stack.pop() {
            if cancel.load(Ordering::Acquire) {
                return batch;
            }
            let Ok(entries) = std::fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                if cancel.load(Ordering::Acquire) {
                    return batch;
                }
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !kind.is_file() || path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                    continue;
                }
                match load(&path, provider, workspaces, cancel, &mut remaining) {
                    Ok(Some((cwds, session))) => {
                        for (workspace, sessions) in workspaces.iter().zip(&mut batch.sessions) {
                            if cwds.iter().any(|cwd| same_dir(cwd, workspace)) {
                                sessions.push(session.clone());
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        if batch.warnings.len() < 8 {
                            batch.warnings.push(format!("{}：{e}", path.display()));
                        }
                    }
                }
            }
        }
    }
    for sessions in &mut batch.sessions {
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    }
    batch
}

fn load(
    path: &Path,
    provider: Provider,
    workspaces: &[PathBuf],
    cancel: &AtomicBool,
    remaining: &mut u64,
) -> Result<Option<Parsed>, String> {
    let stamp = crate::workspace_files::stamp(path)?;
    {
        let mut cache = CACHE.lock().unwrap();
        if let Some(index) = cache
            .iter()
            .position(|c| c.path == path && c.stamp == stamp)
        {
            let item = cache.remove(index).unwrap();
            let parsed = item.parsed.clone();
            cache.push_back(item);
            return Ok(Some(parsed));
        }
    }
    // Most files expose cwd in their first record. Unknown prefixes fall back
    // to the bounded parser rather than silently excluding a valid session.
    let mut prefix = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(64 * 1024)
        .read_to_end(&mut prefix)
        .map_err(|e| e.to_string())?;
    let known = String::from_utf8_lossy(&prefix)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|v| {
            v.get("cwd")
                .or_else(|| v.get("payload").and_then(|v| v.get("cwd")))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    // Codex session_meta owns cwd. Claude can record later cwd changes in a
    // session, so an unrelated prefix is not sufficient to exclude its file.
    if provider == Provider::Codex
        && !known.is_empty()
        && !known
            .iter()
            .any(|cwd| workspaces.iter().any(|w| same_dir(cwd, w)))
    {
        return Ok(None);
    }
    if stamp.len > MAX_FILE {
        return Err("超过 32 MiB，已保留现有对话；请缩小原生历史文件后重试".into());
    }
    if stamp.len > *remaining {
        return Err(format!(
            "本次历史读取超过 {} MiB，已保留现有对话",
            MAX_SCAN / (1024 * 1024)
        ));
    }
    *remaining -= stamp.len;
    let mut lines = Lines {
        reader: BufReader::new(File::open(path).map_err(|e| e.to_string())?),
        read: 0,
        cancel,
        error: None,
    };
    let parsed = match provider {
        Provider::Claude => parse_claude_lines(lines.by_ref()),
        _ => parse_codex_lines(lines.by_ref(), None).map(|(cwd, s)| (vec![cwd], s)),
    };
    if let Some(error) = lines.error {
        return Err(error);
    }
    if crate::workspace_files::stamp(path)? != stamp {
        return Err("扫描期间文件发生变化，跳过此次替换".into());
    }
    if let Some(parsed) = &parsed {
        let bytes = parsed
            .1
            .messages
            .iter()
            .map(|m| m.text.len() + m.detail.len() + m.input.len() + 256)
            .sum::<usize>();
        if bytes <= CACHE_BYTES {
            let mut cache = CACHE.lock().unwrap();
            cache.retain(|c| c.path != path);
            while cache.len() >= 128
                || cache.iter().map(|c| c.bytes).sum::<usize>() + bytes > CACHE_BYTES
            {
                cache.pop_front();
            }
            cache.push_back(Cached {
                path: path.to_owned(),
                stamp,
                parsed: parsed.clone(),
                bytes,
            });
        }
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn isolated_scan_filters_unrelated_files_and_reloads_changed_history() {
        let root = std::env::temp_dir().join(format!(
            "conduit-scan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let history = root.join(".claude/projects/project");
        std::fs::create_dir_all(&history).unwrap();
        let file = history.join("session.jsonl");
        let record = |text: &str| {
            serde_json::json!({"type":"user","promptSource":"sdk","sessionId":"native","cwd":workspace,"message":{"content":text}}).to_string()+"\n"
        };
        std::fs::write(&file, record("first")).unwrap();
        let cancel = AtomicBool::new(false);
        let paths = vec![workspace.clone()];
        assert_eq!(
            run(Some(&root), &paths, &cancel).sessions[0][0].messages[0].text,
            "first"
        );
        let mut budget = 0;
        assert!(
            load(&file, Provider::Claude, &paths, &cancel, &mut budget)
                .unwrap()
                .is_some()
        );
        assert_eq!(budget, 0);
        std::fs::write(&file, record("changed prompt")).unwrap();
        assert_eq!(
            run(Some(&root), &paths, &cancel).sessions[0][0].messages[0].text,
            "changed prompt"
        );
        assert!(run(Some(&root), &[root.join("other")], &cancel).sessions[0].is_empty());
        File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        let oversized = run(Some(&root), &paths, &cancel);
        assert!(oversized.sessions[0].is_empty());
        assert!(
            oversized
                .warnings
                .iter()
                .any(|warning| warning.contains("32 MiB"))
        );
        cancel.store(true, Ordering::Release);
        assert!(run(Some(&root), &paths, &cancel).sessions[0].is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
