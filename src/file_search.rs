//! Workspace file discovery and caret-local @ queries. No file contents enter search.

use std::{
    ops::Range,
    path::Path,
    time::{Duration, Instant},
};

pub const RESULT_LIMIT: usize = 6;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentionQuery {
    pub range: Range<usize>,
    pub text: String,
}

/// Only an explicit @ at a word boundary opens the picker. Offsets are UTF-8.
pub fn query_at(text: &str, selection: Range<usize>) -> Option<MentionQuery> {
    if !selection.is_empty() || !text.is_char_boundary(selection.end) {
        return None;
    }
    let before = text.get(..selection.end)?;
    let start = before.rfind('@')?;
    if before[..start]
        .chars()
        .next_back()
        .is_some_and(|ch| !ch.is_whitespace() && !"([{（【，,：:；;\"'".contains(ch))
    {
        return None;
    }
    let query = &before[start + 1..];
    if query
        .chars()
        .any(|ch| ch.is_whitespace() || "@\"'`<>|".contains(ch))
    {
        return None;
    }
    // Do not replace just the first half of an existing word/path.
    if text[selection.end..]
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_whitespace() && !")]}）】，,。；;：:".contains(ch))
    {
        return None;
    }
    Some(MentionQuery {
        range: start..selection.end,
        text: query.replace('\\', "/"),
    })
}

#[derive(Default)]
pub struct FileIndex {
    pub paths: Vec<String>,
    pub incomplete: bool,
    pub error: Option<String>,
}

pub fn discover(root: &Path) -> FileIndex {
    let mut index = FileIndex::default();
    if let Err(error) = std::fs::read_dir(root) {
        index.error = Some(format!("无法读取工作区：{error}"));
        return index;
    }
    let deadline = Instant::now() + Duration::from_secs(4);
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .follow_links(false)
        .require_git(false)
        .max_depth(Some(64))
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_some_and(|kind| kind.is_dir())
                || !matches!(
                    entry.file_name().to_str(),
                    Some(".git" | ".hg" | ".svn" | "node_modules" | "target" | ".venv")
                )
        })
        .build();
    for (count, entry) in walker.enumerate() {
        if count >= 100_000 || Instant::now() >= deadline {
            index.incomplete = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                index.incomplete = true;
                continue;
            }
        };
        if entry.depth() == 64 && entry.file_type().is_some_and(|kind| kind.is_dir()) {
            index.incomplete = true;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let path = relative.to_string_lossy().replace('\\', "/");
        // The existing reference block is line based.
        if !path.contains(['\n', '\r']) {
            index.paths.push(path);
        }
    }
    index.paths.sort();
    index.paths.dedup();
    index
}

fn score(path: &str, query: &str) -> Option<(u8, usize)> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if query.is_empty() {
        return Some((0, path.matches('/').count()));
    }
    if name == query {
        return Some((0, 0));
    }
    if name.starts_with(query) {
        return Some((1, name.len() - query.len()));
    }
    if let Some(position) = name.find(query) {
        return Some((2, position));
    }
    if let Some(position) = path.find(query) {
        return Some((3, position));
    }
    let mut chars = query.chars();
    let mut next = chars.next();
    let mut first = None;
    for (position, ch) in path.char_indices() {
        if Some(ch) == next {
            first.get_or_insert(position);
            next = chars.next();
            if next.is_none() {
                return Some((4, position - first.unwrap_or(position)));
            }
        }
    }
    None
}

pub fn search(index: &FileIndex, query: &str) -> Vec<String> {
    search_with_limit(index, query, RESULT_LIMIT)
}

pub fn search_with_limit(index: &FileIndex, query: &str, limit: usize) -> Vec<String> {
    let query = query.replace('\\', "/").to_lowercase();
    let mut results = index
        .paths
        .iter()
        .filter_map(|path| score(&path.to_lowercase(), &query).map(|score| (score, path)))
        .collect::<Vec<_>>();
    results.sort_by(|(a, left), (b, right)| {
        a.cmp(b)
            .then_with(|| left.len().cmp(&right.len()))
            .then_with(|| left.cmp(right))
    });
    results
        .into_iter()
        .take(limit)
        .map(|(_, path)| path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_are_caret_local_and_unicode_safe() {
        for text in [
            "@",
            "检查 @src/app",
            "中文\n@文档",
            "看（@文件",
            "看 @src\\app",
        ] {
            assert!(query_at(text, text.len()..text.len()).is_some(), "{text}");
        }
        for text in ["a@host.com", "@@", "\\@src", "@src next", "@src\n", "`@src"] {
            assert!(query_at(text, text.len()..text.len()).is_none(), "{text}");
        }
        let text = "检查 @src 后面的正文";
        let end = "检查 @src".len();
        let query = query_at(text, end..end).unwrap();
        assert_eq!(&text[query.range], "@src");
        assert!(query_at(text, 0..end).is_none());
        assert!(query_at("@src", 2..2).is_none());
        assert!(query_at("中文", 1..1).is_none());
    }

    #[test]
    fn ranking_prefers_names_and_supports_paths_and_fuzzy_queries() {
        let index = FileIndex {
            paths: vec![
                "src/app.rs",
                "docs/app.rs.md",
                "src/app/composer.rs",
                "docs/资料 说明.md",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
            ..Default::default()
        };
        assert_eq!(search(&index, "APP.RS")[0], "src/app.rs");
        assert_eq!(search(&index, "src\\app/")[0], "src/app/composer.rs");
        assert_eq!(search(&index, "cmp")[0], "src/app/composer.rs");
        assert_eq!(search(&index, "资料")[0], "docs/资料 说明.md");
        assert!(search(&index, "unfindable").is_empty());
    }

    #[test]
    fn picker_limit_does_not_change_mention_limit_or_ranking() {
        let index = FileIndex {
            paths: (0..80).map(|i| format!("src/file{i:02}.rs")).collect(),
            ..Default::default()
        };
        let mentions = search(&index, "file");
        let picker = search_with_limit(&index, "file", 50);
        assert_eq!(mentions.len(), RESULT_LIMIT);
        assert_eq!(picker.len(), 50);
        assert_eq!(mentions, picker[..RESULT_LIMIT]);
        assert!(search_with_limit(&index, "file", 0).is_empty());
    }

    #[test]
    fn discovery_respects_ignores_and_includes_hidden_source_files() {
        let root =
            std::env::temp_dir().join(format!("conduit-mention-index-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        for (name, content) in [
            (".gitignore", "ignored.txt\n"),
            ("ignored.txt", "skip"),
            (".env.example", "KEY="),
            ("main.rs", "fn main() {}"),
            (".git/config", ""),
            ("node_modules/pkg/index.js", ""),
        ] {
            std::fs::write(root.join(name), content).unwrap();
        }
        let index = discover(&root);
        assert_eq!(index.paths, vec![".env.example", ".gitignore", "main.rs"]);
        assert!(!index.incomplete);
        assert!(index.error.is_none());
        assert!(discover(&root.join("missing")).error.is_some());
        // Explicitly selecting a generated-directory name still searches that
        // workspace; exclusion applies only to descendants of the chosen root.
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("target/selected.txt"), "text").unwrap();
        assert_eq!(discover(&root.join("target")).paths, vec!["selected.txt"]);
        let mut deep = root.clone();
        for _ in 0..65 {
            deep.push("d");
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("deep.txt"), "text").unwrap();
        assert!(discover(&root).incomplete);
        std::fs::remove_dir_all(root).unwrap();
    }
}
