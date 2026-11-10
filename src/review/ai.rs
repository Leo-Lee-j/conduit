//! Structured reports are suggestions tied to one immutable patch, never proof of correctness.
use super::*;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_REPLY_BYTES: usize = 128 * 1024;
const MAX_REPORTS: usize = 50;
const MAX_PATCH_BYTES: usize = 4 * 1024 * 1024;
const MAX_FINDINGS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
}

impl Priority {
    pub fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::P0 => "review.priority.p0",
            Self::P1 => "review.priority.p1",
            Self::P2 => "review.priority.p2",
            Self::P3 => "review.priority.p3",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Origin {
    pub review: String,
    pub session: u64,
    pub session_created_at: u64,
    pub message: String,
    pub priority: Priority,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rejection {
    Format,
    TooLarge,
    Snapshot,
    Finding(usize),
    Range(usize),
    Storage,
    Incomplete,
    NoReply,
}

impl Rejection {
    pub fn label(&self) -> String {
        match self {
            Self::Finding(index) | Self::Range(index) => conduit::i18n::format(
                if matches!(self, Self::Finding(_)) {
                    "review.ai_bad_finding"
                } else {
                    "review.ai_bad_range"
                },
                &[("index", index.to_string())],
            ),
            _ => problem(match self {
                Self::Format => "review.ai_bad_format",
                Self::TooLarge => "review.ai_reply_limit",
                Self::Snapshot => "review.ai_snapshot_changed",
                Self::Storage => "review.storage_limit",
                Self::Incomplete => "review.ai_incomplete",
                Self::NoReply => "review.ai_no_reply",
                _ => unreachable!(),
            }),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum ReportState {
    #[default]
    Waiting,
    Rejected(Rejection),
    Imported(usize),
    NoFindings,
}

impl ReportState {
    pub fn complete(&self) -> bool {
        matches!(self, Self::Imported(_) | Self::NoFindings)
    }
    pub fn label(&self) -> String {
        match self {
            Self::Waiting => problem("review.ai_waiting"),
            Self::Rejected(reason) => reason.label(),
            Self::Imported(count) => {
                conduit::i18n::format("review.ai_imported", &[("count", count.to_string())])
            }
            Self::NoFindings => problem("review.ai_no_findings"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub key: String,
    pub session: u64,
    pub session_created_at: u64,
    pub target: Target,
    pub patch: String,
    pub fingerprint: String,
    pub state: ReportState,
    pub message: Option<String>,
}

impl Review {
    pub fn capture(target: Target, patch: &str) -> Result<Self, String> {
        if patch.is_empty() || patch.len() > MAX_FEEDBACK_BYTES {
            return Err(problem("review.patch_limit"));
        }
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Ok(Self {
            key: fingerprint(&format!("{}:{now}:{nonce}", std::process::id())),
            session: 0,
            session_created_at: 0,
            target,
            patch: patch.into(),
            fingerprint: fingerprint(patch),
            state: ReportState::Waiting,
            message: None,
        })
    }

    pub fn instructions(&self) -> String {
        let example = serde_json::json!({
            "version": 1, "review_id": self.key, "fingerprint": self.fingerprint,
            "findings": [{"path":self.target.path, "side":"new", "start_line":1, "end_line":1,
                "priority":"P1", "title":"...", "body":"..."}]
        });
        conduit::i18n::format(
            "review.ai_format",
            &[("example", serde_json::to_string_pretty(&example).unwrap())],
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Report {
    version: u32,
    review_id: String,
    fingerprint: String,
    findings: Vec<Finding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    path: String,
    side: WireSide,
    start_line: usize,
    end_line: usize,
    priority: Priority,
    title: String,
    body: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum WireSide {
    Old,
    New,
}

struct Candidate {
    anchor: Anchor,
    priority: Priority,
    title: String,
    body: String,
}

fn parse(review: &Review, text: &str) -> Result<Vec<Candidate>, Rejection> {
    if text.len() > MAX_REPLY_BYTES {
        return Err(Rejection::TooLarge);
    }
    let trimmed = text.trim();
    // Accept a single full JSON response or a single fenced block. Do not search prose,
    // tool output or quoted code for fragments that merely resemble findings.
    let json = if trimmed.starts_with("```") {
        let (opening, rest) = trimmed.split_once('\n').ok_or(Rejection::Format)?;
        if !matches!(opening.trim_end(), "```json" | "```conduit-review" | "```") {
            return Err(Rejection::Format);
        }
        let (body, closing) = rest.rsplit_once('\n').ok_or(Rejection::Format)?;
        if closing.trim() != "```" {
            return Err(Rejection::Format);
        }
        body
    } else {
        trimmed
    };
    let report: Report = serde_json::from_str(json).map_err(|_| Rejection::Format)?;
    if report.version != 1
        || report.review_id != review.key
        || report.fingerprint != review.fingerprint
        || fingerprint(&review.patch) != review.fingerprint
    {
        return Err(Rejection::Snapshot);
    }
    if report.findings.len() > MAX_FINDINGS {
        return Err(Rejection::TooLarge);
    }
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for (index, finding) in report.findings.into_iter().enumerate() {
        let index = index + 1;
        let title = finding.title.trim();
        let body = finding.body.trim();
        if finding.path != review.target.path
            || title.is_empty()
            || title.len() > 256
            || title.chars().any(char::is_control)
            || body.is_empty()
            || body.len() > MAX_COMMENT_BYTES
        {
            return Err(Rejection::Finding(index));
        }
        if finding.start_line == 0 || finding.start_line > finding.end_line {
            return Err(Rejection::Range(index));
        }
        let side = match finding.side {
            WireSide::Old => Side::Original,
            WireSide::New => Side::Modified,
        };
        let anchor = Anchor::capture(&review.patch, side, finding.start_line, finding.end_line)
            .map_err(|_| Rejection::Range(index))?;
        let identity = (
            matches!(side, Side::Original),
            anchor.start,
            anchor.end,
            finding.priority,
            title.to_owned(),
            body.to_owned(),
        );
        if seen.insert(identity) {
            result.push(Candidate {
                anchor,
                priority: finding.priority,
                title: title.into(),
                body: body.into(),
            });
        }
    }
    Ok(result)
}

impl Reviews {
    pub fn can_capture_ai(&self, bytes: usize) -> Result<(), String> {
        let total = self
            .ai
            .iter()
            .try_fold(bytes, |sum, r| sum.checked_add(r.patch.len()));
        if self.ai.len() >= MAX_REPORTS || total.is_none_or(|total| total > MAX_PATCH_BYTES) {
            return Err(problem("review.ai_storage_limit"));
        }
        Ok(())
    }

    /// Validate the whole report and reserve IDs before writing any comment. Repeated delivery
    /// cannot duplicate findings, reopen resolved notes or change a viewed marker.
    pub fn accept_ai(
        &mut self,
        key: &str,
        message: Option<String>,
        reply: Result<&str, Rejection>,
    ) -> Option<String> {
        let position = self
            .ai
            .iter()
            .position(|r| r.key == key && !r.state.complete())?;
        let review = &self.ai[position];
        let candidates = reply.and_then(|text| parse(review, text));
        let candidates = candidates.and_then(|candidates| {
            let last = self
                .comments
                .iter()
                .map(|c| c.id)
                .max()
                .unwrap_or(0)
                .max(self.next_id);
            if self.comments.len().saturating_add(candidates.len()) > MAX_COMMENTS
                || last.checked_add(candidates.len() as u64).is_none()
            {
                return Err(Rejection::Storage);
            }
            Ok((candidates, last))
        });
        let state = match candidates {
            Err(reason) => ReportState::Rejected(reason),
            Ok((candidates, mut last)) => {
                let count = candidates.len();
                for candidate in candidates {
                    last += 1;
                    self.comments.push(Comment {
                        id: last,
                        target: review.target.clone(),
                        anchor: candidate.anchor,
                        text: candidate.body,
                        status: Status::NeedsReview,
                        ai: Some(Origin {
                            review: review.key.clone(),
                            session: review.session,
                            session_created_at: review.session_created_at,
                            message: message.clone().unwrap_or_default(),
                            priority: candidate.priority,
                            title: candidate.title,
                        }),
                    });
                }
                self.next_id = last;
                if count == 0 {
                    ReportState::NoFindings
                } else {
                    ReportState::Imported(count)
                }
            }
        };
        let label = state.label();
        self.ai[position].state = state;
        self.ai[position].message = message;
        Some(label)
    }
}

#[cfg(test)]
mod tests;
