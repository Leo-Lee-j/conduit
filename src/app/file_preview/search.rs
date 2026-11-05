//! Literal and bounded subsequence matches, retaining original UTF-8 offsets.
use std::ops::Range;

const LIMIT: usize = 2_000;
const MAX_GAP: usize = 32;
pub(super) const MAX_QUERY_CHARS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Match {
    pub range: Range<usize>,
    pub spans: Vec<Range<usize>>,
}

#[derive(Default)]
pub(super) struct Results {
    pub hits: Vec<Match>,
    pub truncated: bool,
}

fn folded(text: &str, offset: usize) -> Vec<(char, Range<usize>)> {
    text.char_indices()
        .flat_map(|(i, ch)| {
            ch.to_lowercase()
                .map(move |lower| (lower, offset + i..offset + i + ch.len_utf8()))
        })
        .collect()
}

fn hit(chars: &[(char, Range<usize>)], indices: impl Iterator<Item = usize>) -> Match {
    let mut spans: Vec<Range<usize>> = Vec::new();
    for index in indices {
        let range = &chars[index].1;
        if let Some(last) = spans.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            spans.push(range.clone());
        }
    }
    Match {
        range: spans.first().unwrap().start..spans.last().unwrap().end,
        spans,
    }
}

pub(super) fn search(text: &str, query: &str, fuzzy: bool) -> Results {
    let mut results = Results::default();
    if query.trim().is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return results;
    }
    let query: Vec<_> = query.chars().flat_map(char::to_lowercase).collect();
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
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let chars = folded(line.trim_end_matches(['\r', '\n']), offset);
        offset += line.len();
        let before = results.hits.len();
        let mut matched = 0;
        for (i, (ch, _)) in chars.iter().enumerate() {
            while matched > 0 && *ch != query[matched] {
                matched = failure[matched - 1];
            }
            if *ch == query[matched] {
                matched += 1;
            }
            if matched == query.len() {
                let found = hit(&chars, i + 1 - matched..i + 1);
                if results
                    .hits
                    .last()
                    .is_none_or(|last| last.range != found.range)
                {
                    results.hits.push(found);
                }
                matched = 0;
                if results.hits.len() > LIMIT {
                    results.hits.truncate(LIMIT);
                    results.truncated = true;
                    return results;
                }
            }
        }
        // Prefer literal occurrences on a line; fuzzy matches never cross lines.
        if !fuzzy || results.hits.len() != before {
            continue;
        }
        let mut start = 0;
        while start < chars.len() {
            if chars[start].0 != query[0] {
                start += 1;
                continue;
            }
            let mut indices = vec![start];
            let mut next = start + 1;
            for ch in &query[1..] {
                let end = (next + MAX_GAP + 1).min(chars.len());
                let Some(index) = (next..end).find(|&i| chars[i].0 == *ch) else {
                    break;
                };
                indices.push(index);
                next = index + 1;
            }
            if indices.len() == query.len() {
                results.hits.push(hit(&chars, indices.into_iter()));
                start = next;
                if results.hits.len() > LIMIT {
                    results.hits.truncate(LIMIT);
                    results.truncated = true;
                    return results;
                }
            } else {
                start += 1;
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_unicode_offsets_and_multiple_occurrences() {
        let text = "前İ后 i\n中文 中文\n";
        assert_eq!(
            search(text, "i", false)
                .hits
                .iter()
                .map(|h| h.range.clone())
                .collect::<Vec<_>>(),
            [3..5, 9..10]
        );
        let hits = search(text, "中文", false).hits;
        assert_eq!(hits.len(), 2);
        for found in hits {
            assert_eq!(&text[found.range], "中文");
        }
        assert!(search(text, "  ", true).hits.is_empty());
    }

    #[test]
    fn fuzzy_is_bounded_and_preserves_matched_characters() {
        let text = "open_workspace_file\nopfile opfile\n文件内容搜索\nop\nfile\n";
        let hits = search(text, "opfile", true).hits;
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0]
                .spans
                .iter()
                .map(|r| &text[r.clone()])
                .collect::<String>(),
            "opfile"
        );
        assert_eq!(search(text, "opfile", false).hits.len(), 2);
        assert_eq!(search(text, "文件搜索", true).hits.len(), 1);
        assert!(
            search(&format!("a{}b", "x".repeat(40)), "ab", true)
                .hits
                .is_empty()
        );
    }

    #[test]
    fn results_are_capped_in_document_order() {
        let result = search(&"a ".repeat(LIMIT + 1), "a", true);
        assert_eq!(result.hits.len(), LIMIT);
        assert!(result.truncated);
        assert!(
            result
                .hits
                .windows(2)
                .all(|pair| pair[0].range.start < pair[1].range.start)
        );
    }

    #[test]
    fn queries_preserve_whitespace_and_enforce_the_character_limit() {
        let text = "a target b\r\ntarget\r\na target b\n";
        let hits = search(text, " target ", false).hits;
        assert_eq!(hits.len(), 2);
        for found in hits {
            assert_eq!(&text[found.range], " target ");
        }
        let query = "İ".repeat(MAX_QUERY_CHARS);
        assert_eq!(search(&query, &query, false).hits.len(), 1);
        let query = format!("{query}İ");
        assert!(search(&query, &query, false).hits.is_empty());
    }

    #[test]
    fn exact_limit_and_unicode_expansions_do_not_report_extra_matches() {
        let results = search(&"a ".repeat(LIMIT), "a", true);
        assert_eq!(results.hits.len(), LIMIT);
        assert!(!results.truncated);
        let text = "İİ\r\n中文\r\n😀 中文\n";
        assert_eq!(
            search(text, "i\u{307}", false)
                .hits
                .iter()
                .map(|hit| hit.range.clone())
                .collect::<Vec<_>>(),
            [0..2, 2..4]
        );
        for hit in search(text, "中文", true).hits {
            assert_eq!(&text[hit.range], "中文");
        }
        assert!(search(text, "文中", true).hits.is_empty());
    }
}
