//! Persistent local review notes. Locations refer to a captured diff, never live line numbers.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub mod ai;
pub mod remote;

pub const MAX_COMMENT_BYTES: usize = 8 * 1024;
pub const MAX_SNIPPET_BYTES: usize = 32 * 1024;
pub const MAX_FEEDBACK_BYTES: usize = 128 * 1024;
const MAX_COMMENTS: usize = 1000;
const MAX_VIEWED: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Original,
    Modified,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    #[default]
    Open,
    NeedsReview,
    Resolved,
    Dismissed,
}

impl Status {
    pub const ALL: [Self; 4] = [
        Self::Open,
        Self::NeedsReview,
        Self::Resolved,
        Self::Dismissed,
    ];
    pub fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Open => "review.status.open",
            Self::NeedsReview => "review.status.needs_review",
            Self::Resolved => "review.status.resolved",
            Self::Dismissed => "review.status.dismissed",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub workspace: PathBuf,
    pub path: String,
    /// Comparison source (index/worktree/commit/full ref IDs), independent of translated labels.
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub side: Side,
    pub start: usize,
    pub end: usize,
    pub snippet: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub target: Target,
    pub anchor: Anchor,
    pub text: String,
    pub status: Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<ai::Origin>,
}

impl Comment {
    pub fn display_text(&self) -> String {
        match &self.ai {
            Some(origin) => format!(
                "AI · {} · {}\n{}",
                origin.priority.label(),
                origin.title,
                self.text
            ),
            None => self.text.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Viewed {
    pub target: Target,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reviews {
    #[serde(default)]
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub viewed: Vec<Viewed>,
    #[serde(default)]
    pub ai: Vec<ai::Review>,
    #[serde(default)]
    pub remote: Vec<remote::SavedThread>,
    #[serde(default)]
    next_id: u64,
}

pub fn fingerprint(patch: &str) -> String {
    format!("{:x}", Sha256::digest(patch.as_bytes()))
}

fn problem(key: &'static str) -> String {
    conduit::i18n::text_static(key).to_owned()
}

impl Anchor {
    /// Capture only contiguous source lines supplied by this patch. Never read a newer file.
    pub fn capture(patch: &str, side: Side, start: usize, end: usize) -> Result<Self, String> {
        let (start, end) = (start.min(end), start.max(end));
        if start == 0 || end.saturating_sub(start) >= 200 {
            return Err(problem("review.range_limit"));
        }
        let mut old = 0usize;
        let mut new = 0usize;
        let mut in_hunk = false;
        let mut expected = start;
        let mut snippet = String::new();
        for line in patch.lines() {
            if let Some(header) = line.strip_prefix("@@ ") {
                let mut fields = header.split_whitespace();
                let parse = |value: Option<&str>, prefix| {
                    value?
                        .strip_prefix(prefix)?
                        .split(',')
                        .next()?
                        .parse::<usize>()
                        .ok()
                };
                match (parse(fields.next(), '-'), parse(fields.next(), '+')) {
                    (Some(a), Some(b)) => {
                        old = a;
                        new = b;
                        in_hunk = true;
                    }
                    _ => return Err(problem("review.invalid_range")),
                }
                continue;
            }
            if !in_hunk {
                continue;
            }
            let (has_old, has_new) = match line.as_bytes().first() {
                Some(b' ') => (true, true),
                Some(b'-') => (true, false),
                Some(b'+') => (false, true),
                Some(b'\\') => continue,
                _ => {
                    in_hunk = false;
                    continue;
                }
            };
            let number = match side {
                Side::Original if has_old => Some(old),
                Side::Modified if has_new => Some(new),
                _ => None,
            };
            if let Some(number) = number.filter(|n| *n >= start && *n <= end) {
                if number != expected {
                    return Err(problem("review.invalid_range"));
                }
                if !snippet.is_empty() || expected != start {
                    snippet.push('\n');
                }
                snippet.push_str(&line[1..]);
                if snippet.len() > MAX_SNIPPET_BYTES {
                    return Err(problem("review.range_limit"));
                }
                expected += 1;
            }
            old += usize::from(has_old);
            new += usize::from(has_new);
            if expected > end {
                break;
            }
        }
        if expected <= end {
            return Err(problem("review.invalid_range"));
        }
        Ok(Self {
            side,
            start,
            end,
            snippet,
            fingerprint: fingerprint(patch),
        })
    }
}

impl Reviews {
    pub fn add(&mut self, target: Target, anchor: Anchor, text: &str) -> Result<u64, String> {
        let text = text.trim();
        if text.is_empty() || text.len() > MAX_COMMENT_BYTES {
            return Err(problem("review.comment_limit"));
        }
        if self.comments.len() >= MAX_COMMENTS {
            return Err(problem("review.storage_limit"));
        }
        let last = self
            .comments
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(0)
            .max(self.next_id);
        let id = last
            .checked_add(1)
            .ok_or_else(|| problem("review.storage_limit"))?;
        self.next_id = id;
        self.comments.push(Comment {
            id,
            target,
            anchor,
            text: text.to_owned(),
            status: Status::Open,
            ai: None,
        });
        Ok(id)
    }

    pub fn is_viewed(&self, target: &Target, fingerprint: &str) -> bool {
        self.viewed
            .iter()
            .any(|v| &v.target == target && v.fingerprint == fingerprint)
    }

    pub fn set_viewed(&mut self, target: Target, fingerprint: String, viewed: bool) {
        self.viewed.retain(|v| v.target != target);
        if viewed {
            if self.viewed.len() >= MAX_VIEWED {
                self.viewed.remove(0);
            }
            self.viewed.push(Viewed {
                target,
                fingerprint,
            });
        }
    }

    /// Include original evidence and explicit staleness; attachment alone changes no review status.
    pub fn feedback(&self, target: &Target, ids: &[u64], current: &str) -> Result<String, String> {
        let selected: Vec<_> = self
            .comments
            .iter()
            .filter(|c| &c.target == target && ids.contains(&c.id))
            .collect();
        if selected.is_empty() {
            return Err(problem("review.select_feedback"));
        }
        let mut output = conduit::i18n::format(
            "review.feedback_intro",
            &[
                ("path", target.path.clone()),
                ("source", target.source.clone()),
                ("workspace", target.workspace.display().to_string()),
            ],
        );
        for comment in selected {
            let side = conduit::i18n::text_static(match comment.anchor.side {
                Side::Original => "review.original",
                Side::Modified => "review.modified",
            });
            let freshness = conduit::i18n::text_static(if current.is_empty() {
                "review.unverified"
            } else if comment.anchor.fingerprint == current {
                "review.current"
            } else {
                "review.outdated"
            });
            output.push_str(&format!(
                "\n\n#{} · {side} {}–{} · {freshness}\n{}\n",
                comment.id,
                comment.anchor.start,
                comment.anchor.end,
                comment.display_text()
            ));
            if let Some(origin) = &comment.ai {
                output.push_str(&conduit::i18n::format(
                    "review.ai_source",
                    &[("session", origin.session.to_string())],
                ));
                output.push('\n');
            }
            // Quoted source cannot close a Markdown code fence supplied by the user.
            for line in comment.anchor.snippet.split('\n') {
                output.push_str("    ");
                output.push_str(line);
                output.push('\n');
            }
            if output.len() > MAX_FEEDBACK_BYTES {
                return Err(problem("review.feedback_limit"));
            }
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PATCH: &str = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -10,3 +10,4 @@\n keep\n-old\n+新行\n+second\n end\n";
    fn target() -> Target {
        Target {
            workspace: PathBuf::from("C:/repo"),
            path: "a.rs".into(),
            source: "unstaged".into(),
        }
    }
    #[test]
    fn selections_capture_the_correct_side_and_reject_missing_context() {
        let a = Anchor::capture(PATCH, Side::Original, 11, 11).unwrap();
        assert_eq!(a.snippet, "old");
        assert_eq!(
            Anchor::capture(PATCH, Side::Modified, 12, 11)
                .unwrap()
                .snippet,
            "新行\nsecond"
        );
        assert!(Anchor::capture(PATCH, Side::Modified, 1, 11).is_err());
        assert!(Anchor::capture(PATCH, Side::Modified, 1, 300).is_err());
        let gaps = "@@ -1 +1 @@\n a\n@@ -8 +8 @@\n b\n";
        assert!(Anchor::capture(gaps, Side::Modified, 1, 8).is_err());
    }
    #[test]
    fn review_survives_serialization_and_changed_code_never_reuses_viewed_or_location() {
        let mut reviews = Reviews::default();
        let anchor = Anchor::capture(PATCH, Side::Modified, 11, 12).unwrap();
        let digest = anchor.fingerprint.clone();
        let id = reviews.add(target(), anchor, "检查空输入").unwrap();
        reviews.set_viewed(target(), digest.clone(), true);
        let encoded = serde_json::to_string(&reviews).unwrap();
        let restored: Reviews = serde_json::from_str(&encoded).unwrap();
        assert!(restored.is_viewed(&target(), &digest));
        assert!(!restored.is_viewed(&target(), &fingerprint(&PATCH.replace("新行", "changed"))));
        let text = restored.feedback(&target(), &[id], "changed").unwrap();
        assert!(text.contains(conduit::i18n::text_static("review.outdated")));
        assert!(text.contains("新行"));
        assert_eq!(restored.comments[0].status, Status::Open);
        let mut other = target();
        other.workspace = PathBuf::from("C:/other");
        assert!(restored.feedback(&other, &[id], &digest).is_err());
        other = target();
        other.source = "staged".into();
        assert!(restored.feedback(&other, &[id], &digest).is_err());
    }
    #[test]
    fn limits_do_not_drop_existing_notes_and_identifiers_do_not_repeat_after_deletion() {
        let mut reviews = Reviews::default();
        let a = Anchor::capture(PATCH, Side::Original, 11, 11).unwrap();
        let first = reviews.add(target(), a.clone(), "first").unwrap();
        assert!(
            reviews
                .add(target(), a.clone(), &"x".repeat(MAX_COMMENT_BYTES + 1))
                .is_err()
        );
        assert_eq!(reviews.comments.len(), 1);
        reviews.comments.clear();
        assert!(reviews.add(target(), a, "second").unwrap() > first);
    }
}
