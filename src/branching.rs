//! Native branch boundaries are explicit, completed turns, never guessed from message counts.
use crate::domain::{ItemKind, Provider, SessionRecord};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_POINTS: usize = 256;
const MAX_HISTORY_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForkRequest {
    pub thread_id: String,
    pub last_turn_id: String,
}

impl ForkRequest {
    pub fn valid(&self) -> bool {
        [&self.thread_id, &self.last_turn_id]
            .iter()
            .all(|id| !id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Point {
    pub request: ForkRequest,
    pub through: usize,
    pub last_key: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Origin {
    pub session: u64,
    pub point: Point,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Branching {
    pub points: Vec<Point>,
    pub origin: Option<Origin>,
}

fn error(key: &'static str) -> String {
    conduit::i18n::text(key)
}

fn fingerprint(session: &SessionRecord, through: usize) -> Option<String> {
    let messages = session.messages.get(..through)?;
    let mut bytes = 0usize;
    let mut hash = Sha256::new();
    for item in messages {
        // Timing and tool status can finish after the last protocol event. The visible
        // content and identity must match; presentation-only updates do not invalidate it.
        let kind = match item.kind {
            ItemKind::User => "user",
            ItemKind::Assistant => "assistant",
            ItemKind::Reasoning => "reasoning",
            ItemKind::Tool => "tool",
            ItemKind::Notice => "notice",
            ItemKind::Error => "error",
        };
        for value in [kind, &item.key, &item.text, &item.input, &item.detail] {
            bytes = bytes.checked_add(value.len())?;
            if bytes > MAX_HISTORY_BYTES {
                return None;
            }
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
    }
    Some(format!("{:x}", hash.finalize()))
}

pub fn record(session: &mut SessionRecord, thread_id: String, turn_id: String) {
    let request = ForkRequest {
        thread_id,
        last_turn_id: turn_id,
    };
    if session.provider != Provider::Codex
        || session.native_id.as_deref() != Some(&request.thread_id)
        || !request.valid()
        || !session.transcript.loaded
    {
        return;
    }
    let Some(last) = session.messages.last() else {
        return;
    };
    let through = session.messages.len();
    let Some(fingerprint) = fingerprint(session, through) else {
        return;
    };
    let point = Point {
        request,
        through,
        last_key: last.key.clone(),
        fingerprint,
    };
    session
        .branching
        .points
        .retain(|p| p.request != point.request);
    session.branching.points.push(point);
    if session.branching.points.len() > MAX_POINTS {
        let extra = session.branching.points.len() - MAX_POINTS;
        session.branching.points.drain(..extra);
    }
}

pub fn point_at(session: &SessionRecord, through: usize) -> Result<&Point, String> {
    if session.provider != Provider::Codex {
        return Err(error("branching.unsupported"));
    }
    if !session.transcript.loaded || session.interrupted {
        return Err(error("branching.wait"));
    }
    session
        .branching
        .points
        .iter()
        .rev()
        .find(|p| {
            p.through == through
                && session.native_id.as_deref() == Some(&p.request.thread_id)
                && through
                    .checked_sub(1)
                    .and_then(|i| session.messages.get(i))
                    .is_some_and(|m| m.key == p.last_key)
        })
        .ok_or_else(|| error("branching.no_boundary"))
}

pub fn prepare(
    source: &SessionRecord,
    through: usize,
    id: u64,
    now: u64,
) -> Result<SessionRecord, String> {
    let point = point_at(source, through)?.clone();
    if fingerprint(source, through).as_deref() != Some(&point.fingerprint) {
        return Err(error("branching.changed"));
    }
    let mut session = source.clone();
    session.id = id;
    session.created_at = now;
    session.title = conduit::i18n::format("branching.title", &[("title", source.title.clone())]);
    session.archived = false;
    session.pinned = false;
    session.native_id = None;
    session.exit_code = None;
    session.interrupted = false;
    session.usage = Default::default();
    session.messages.truncate(through);
    session.transcript = Default::default();
    session.branching = Branching {
        points: vec![],
        origin: Some(Origin {
            session: source.id,
            point,
        }),
    };
    Ok(session)
}

pub fn pending(session: &SessionRecord) -> bool {
    session.native_id.is_none() && session.branching.origin.is_some()
}

pub fn request(session: &SessionRecord) -> Result<ForkRequest, String> {
    let origin = session
        .branching
        .origin
        .as_ref()
        .ok_or_else(|| error("branching.changed"))?;
    if session.provider != Provider::Codex
        || !pending(session)
        || !session.transcript.loaded
        || !origin.point.request.valid()
        || fingerprint(session, origin.point.through).as_deref() != Some(&origin.point.fingerprint)
        || session
            .messages
            .get(origin.point.through..)
            .is_none_or(|tail| {
                tail.iter()
                    .any(|m| !matches!(m.kind, ItemKind::Notice | ItemKind::Error))
            })
    {
        return Err(error("branching.changed"));
    }
    Ok(origin.point.request.clone())
}

pub fn bound(session: &mut SessionRecord, native_id: &str) {
    if pending(session)
        && let Some(origin) = &session.branching.origin
    {
        // The protocol verified that this exact turn is present in the new native thread.
        let mut point = origin.point.clone();
        point.request.thread_id = native_id.to_owned();
        session.branching.points = vec![point];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SessionRecord {
        serde_json::from_value(serde_json::json!({
            "id": 1, "provider": "Codex", "created_at": 1, "exit_code": null,
            "native_id": "source", "title": "source", "messages": [
                {"key":"turn-0-user", "kind":"User", "text":"first"},
                {"key":"turn-0-reply", "kind":"Assistant", "text":"answer"}
            ]
        }))
        .unwrap()
    }

    #[test]
    fn native_branches_keep_the_exact_prefix_and_reject_drift_or_unmapped_boundaries() {
        let mut source = source();
        record(&mut source, "other".into(), "turn-1".into());
        assert!(source.branching.points.is_empty());
        record(&mut source, "source".into(), "turn-1".into());
        assert!(prepare(&source, 1, 2, 100).is_err());
        let mut branch = prepare(&source, 2, 2, 100).unwrap();
        assert!(pending(&branch));
        assert_eq!(request(&branch).unwrap().last_turn_id, "turn-1");
        assert_eq!(source.native_id.as_deref(), Some("source"));
        branch.messages[0].text = "local edit".into();
        assert!(request(&branch).is_err());
        assert_eq!(source.messages[0].text, "first");
        source.messages[1].elapsed_ms = Some(5);
        let mut branch = prepare(&source, 2, 3, 100).unwrap();
        bound(&mut branch, "fork");
        branch.native_id = Some("fork".into());
        let restored: SessionRecord =
            serde_json::from_value(serde_json::to_value(&branch).unwrap()).unwrap();
        assert_eq!(point_at(&restored, 2).unwrap().request.thread_id, "fork");
        assert!(request(&restored).is_err());
        source.messages[1].text = "changed".into();
        assert!(prepare(&source, 2, 4, 100).is_err());
    }
}
