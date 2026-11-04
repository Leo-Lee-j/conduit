//! Query work runs on snapshots. Rendering only reads completed results.
use super::search::SearchHit;
use crate::domain::{ItemKind, Workspace};
use std::{collections::VecDeque, ops::Range, time::Instant};

pub(super) struct SearchUi {
    pub hits: Vec<SearchHit>,
    pub generation: u64,
    pub revision: u64,
    pub requested: Instant,
    pub task: Option<gpui_kit::Task<()>>,
}
impl Default for SearchUi {
    fn default() -> Self {
        Self {
            hits: Vec::new(),
            generation: 0,
            revision: 0,
            requested: Instant::now(),
            task: None,
        }
    }
}

/// KMP over Unicode-lowercased bytes, retaining only one query-sized offset
/// window. A case expansion still maps to the complete original character.
struct Matcher {
    query: Vec<u8>,
    failure: Vec<usize>,
}
impl Matcher {
    fn new(query: &str) -> Self {
        let query = query.to_lowercase().into_bytes();
        let mut failure = vec![0; query.len()];
        let mut matched = 0;
        for i in 1..query.len() {
            while matched > 0 && query[i] != query[matched] {
                matched = failure[matched - 1];
            }
            if query[i] == query[matched] {
                matched += 1;
            }
            failure[i] = matched;
        }
        Self { query, failure }
    }
    fn first(&self, text: &str) -> Option<Range<usize>> {
        if self.query.is_empty() {
            return None;
        }
        let mut positions = VecDeque::with_capacity(self.query.len());
        let mut matched = 0;
        for (index, ch) in text.char_indices() {
            for lower in ch.to_lowercase() {
                let mut encoded = [0; 4];
                for byte in lower.encode_utf8(&mut encoded).bytes() {
                    if positions.len() == self.query.len() {
                        positions.pop_front();
                    }
                    positions.push_back(index);
                    while matched > 0 && byte != self.query[matched] {
                        matched = self.failure[matched - 1];
                    }
                    if byte == self.query[matched] {
                        matched += 1;
                    }
                    if matched == self.query.len() {
                        return Some(*positions.front()?..index + ch.len_utf8());
                    }
                }
            }
        }
        None
    }
}

fn snippet_at(text: &str, found: Option<Range<usize>>) -> String {
    let (start, end) = if let Some(found) = found {
        let start = text[..found.start]
            .char_indices()
            .rev()
            .nth(39)
            .map_or(0, |(i, _)| i);
        let end = text[found.end..]
            .char_indices()
            .nth(120)
            .map_or(text.len(), |(i, _)| found.end + i);
        (start, end)
    } else {
        (
            0,
            text.char_indices().nth(160).map_or(text.len(), |(i, _)| i),
        )
    };
    let mut result = text[start..end].replace(['\r', '\n'], " ");
    if start > 0 {
        result.insert(0, '…');
    }
    if end < text.len() {
        result.push('…');
    }
    result
}
#[cfg(test)]
pub(super) fn snippet(text: &str, query: &str) -> String {
    snippet_at(text, Matcher::new(query).first(text))
}

pub(super) fn collect(workspaces: &[Workspace], query: &str) -> Vec<SearchHit> {
    let matcher = Matcher::new(query);
    let mut hits = Vec::new();
    for workspace in workspaces {
        let name = workspace.name();
        for session in &workspace.sessions {
            let title_match = matcher.first(&session.title).is_some();
            let message = if !title_match && !query.is_empty() {
                session
                    .messages
                    .iter()
                    .find_map(|m| matcher.first(&m.text).map(|range| (m, Some(range))))
            } else {
                None
            };
            if !query.is_empty() && !title_match && message.is_none() {
                continue;
            }
            let source = message.or_else(|| {
                session
                    .messages
                    .iter()
                    .find(|m| m.kind == ItemKind::User)
                    .or_else(|| session.messages.first())
                    .map(|m| (m, None))
            });
            hits.push(SearchHit {
                session: session.id,
                provider: session.provider,
                workspace_name: name.clone(),
                title: if session.title.is_empty() {
                    "新会话".into()
                } else {
                    session.title.clone()
                },
                archived: session.archived,
                snippet: source.map_or_else(String::new, |(m, range)| snippet_at(&m.text, range)),
            });
            if hits.len() == 40 {
                return hits;
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matching_preserves_unicode_expansion_and_bounds_snippet_allocations() {
        assert_eq!(Matcher::new("i").first("İ"), Some(0..2));
        assert_eq!(Matcher::new("中").first("a中文"), Some(1..4));
        let text = format!("{}needle{}", "前".repeat(100_000), "后".repeat(100_000));
        let preview = snippet(&text, "NEEDLE");
        assert!(preview.contains("needle"));
        assert!(preview.chars().count() <= 168);
        assert!(preview.starts_with('…') && preview.ends_with('…'));
    }
}
