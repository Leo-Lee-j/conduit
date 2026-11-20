//! Bounded differences between two successful reads, never inferred tool timing.
use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;

pub(in crate::app) const MAX_REVIEW_BYTES: usize = 256 * 1024;
const MAX_LCS_CELLS: usize = 1_000_000;
const MAX_HUNKS: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct Hunk {
    pub old: Range<usize>,
    pub new: Range<usize>,
    pub line: usize,
    pub added: usize,
    pub removed: usize,
}

#[derive(Clone, Debug)]
pub(in crate::app) struct ChangeSet {
    pub before: Arc<str>,
    pub after: Arc<str>,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub removed: usize,
}

impl ChangeSet {
    /// None means absent/oversized/over-budget evidence: only a file-level notice is safe.
    pub fn between(before: &str, after: &str) -> Option<Self> {
        if before == after || before.len() + after.len() > MAX_REVIEW_BYTES {
            return None;
        }
        let old: Vec<_> = before.split_inclusive('\n').collect();
        let new: Vec<_> = after.split_inclusive('\n').collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let a = &old[prefix..old.len() - suffix];
        let b = &new[prefix..new.len() - suffix];
        let columns = b.len() + 1;
        let cells = (a.len() + 1).checked_mul(columns)?;
        if cells > MAX_LCS_CELLS {
            return None;
        }
        let mut lcs = vec![0u16; cells];
        for i in (0..a.len()).rev() {
            for j in (0..b.len()).rev() {
                lcs[i * columns + j] = if a[i] == b[j] {
                    lcs[(i + 1) * columns + j + 1] + 1
                } else {
                    lcs[(i + 1) * columns + j].max(lcs[i * columns + j + 1])
                };
            }
        }
        let offsets = |lines: &[&str]| {
            let mut offsets = vec![0];
            for line in lines {
                offsets.push(offsets.last().unwrap() + line.len());
            }
            offsets
        };
        let old_offsets = offsets(&old);
        let new_offsets = offsets(&new);
        let (mut i, mut j) = (0, 0);
        let mut hunks = Vec::new();
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && a[i] == b[j] {
                i += 1;
                j += 1;
                continue;
            }
            let (start_i, start_j) = (i, j);
            while i < a.len() || j < b.len() {
                if i < a.len() && j < b.len() && a[i] == b[j] {
                    break;
                }
                if j < b.len()
                    && (i == a.len() || lcs[i * columns + j + 1] > lcs[(i + 1) * columns + j])
                {
                    j += 1;
                } else {
                    i += 1;
                }
            }
            hunks.push(Hunk {
                old: old_offsets[prefix + start_i]..old_offsets[prefix + i],
                new: new_offsets[prefix + start_j]..new_offsets[prefix + j],
                line: prefix + start_j + 1,
                added: j - start_j,
                removed: i - start_i,
            });
            if hunks.len() > MAX_HUNKS {
                return None;
            }
        }
        Some(Self {
            before: before.into(),
            after: after.into(),
            added: hunks.iter().map(|h| h.added).sum(),
            removed: hunks.iter().map(|h| h.removed).sum(),
            hunks,
        })
    }

    /// One bounded replay of the changed span. Grapheme boundaries keep emoji,
    /// combining marks and CRLF intact; this is not a recording of keystrokes.
    pub fn replay(&self) -> Replay {
        let old: Vec<_> = self.before.grapheme_indices(true).collect();
        let new: Vec<_> = self.after.grapheme_indices(true).collect();
        let prefix = old.iter().zip(&new).take_while(|(a, b)| a.1 == b.1).count();
        let suffix = old[prefix..]
            .iter()
            .rev()
            .zip(new[prefix..].iter().rev())
            .take_while(|(a, b)| a.1 == b.1)
            .count();
        let start = old.get(prefix).map_or(self.before.len(), |g| g.0);
        let old_end = old
            .get(old.len() - suffix)
            .map_or(self.before.len(), |g| g.0);
        let new_end = new
            .get(new.len() - suffix)
            .map_or(self.after.len(), |g| g.0);
        let mut boundaries: Vec<_> = new[prefix..new.len() - suffix]
            .iter()
            .map(|g| g.0)
            .collect();
        boundaries.push(new_end);
        Replay {
            start,
            old_end,
            boundaries,
        }
    }
}

pub(in crate::app) struct Replay {
    pub start: usize,
    pub old_end: usize,
    boundaries: Vec<usize>,
}
impl Replay {
    pub fn frame(&self, changes: &ChangeSet, step: usize, total: usize) -> (String, usize) {
        if step == 0 {
            return (changes.before.to_string(), self.start);
        }
        let index = (self.boundaries.len() - 1) * step.min(total) / total.max(1);
        let end = self.boundaries[index];
        (
            format!(
                "{}{}{}",
                &changes.after[..self.start],
                &changes.after[self.start..end],
                &changes.before[self.old_end..]
            ),
            end,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn content_review_exact_hunks_preserve_separate_edits_deletions_and_line_endings() {
        let c =
            ChangeSet::between("one\r\nold\r\nsame\r\ngone\r\n", "one\r\nnew\r\nsame\r\n").unwrap();
        assert_eq!((c.added, c.removed, c.hunks.len()), (1, 2, 2));
        assert_eq!(&c.after[c.hunks[0].new.clone()], "new\r\n");
        assert!(c.hunks[1].new.is_empty());
        let mut replayed = c.before.to_string();
        for h in c.hunks.iter().rev() {
            replayed.replace_range(h.old.clone(), &c.after[h.new.clone()]);
        }
        assert_eq!(replayed, c.after.as_ref());
        assert!(ChangeSet::between("same", "same").is_none());
        assert!(ChangeSet::between("", "new").is_some());
        assert!(
            ChangeSet::between("gone", "").unwrap().hunks[0]
                .new
                .is_empty()
        );
    }
    #[test]
    fn content_review_playback_respects_graphemes_and_ends_at_exact_snapshot() {
        let c = ChangeSet::between("前👨‍👩‍👧‍👦旧e\u{301}\r\n尾", "前👨‍👩‍👧‍👦新🧑🏽‍💻e\u{301}\r\n尾").unwrap();
        let replay = c.replay();
        for step in 0..=60 {
            let (frame, caret) = replay.frame(&c, step, 60);
            assert!(frame.is_char_boundary(caret));
            assert!(!frame.ends_with('\u{200d}'));
            if step > 0 {
                assert!(
                    c.after.grapheme_indices(true).any(|g| g.0 == caret) || caret == c.after.len()
                );
            }
        }
        assert_eq!(replay.frame(&c, 60, 60).0, c.after.as_ref());
        assert!(ChangeSet::between("", &"x".repeat(MAX_REVIEW_BYTES + 1)).is_none());
        assert!(ChangeSet::between(&"a\n".repeat(1100), &"b\n".repeat(1100)).is_none());
    }
}
