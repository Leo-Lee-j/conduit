//! Per-open-document baselines. Proposals never replace the disk snapshot.
use super::super::file_preview::{changes::MAX_REVIEW_BYTES, proposal::Snapshot};
use super::*;
use crate::protocol::file_proposals::{Input, Update};

#[derive(Default)]
pub(super) struct Pending {
    attempts: Vec<Attempt>,
}
struct Attempt {
    session: u64,
    key: String,
    before: SharedString,
    stamp: Stamp,
    started: Instant,
    blocked: bool,
    snapshot: Option<Snapshot>,
}
impl Pending {
    fn clear(&mut self) {
        self.attempts.clear();
    }
    fn current(&self) -> Option<Snapshot> {
        self.attempts
            .iter()
            .find(|a| !a.blocked && a.snapshot.is_some())
            .and_then(|a| a.snapshot.clone())
    }
    fn end(&mut self, session: u64, key: Option<&str>) {
        self.attempts
            .retain(|a| a.session != session || key.is_some_and(|key| a.key != key));
    }
}
fn proposed(
    input: &Input,
    root: &Path,
    key: &FileKey,
    before: &str,
) -> Option<(String, usize, usize)> {
    let path = Path::new(&input.path);
    let target = observations::normalized(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    })?;
    if target != observations::normalized(&key.workspace.join(&key.relative))? {
        return None;
    }
    let (after, start, end) = if let Some(old) = &input.old {
        if old.is_empty() || before.matches(old).count() != 1 {
            return None;
        }
        let start = before.find(old)?;
        let after = before.replacen(old, &input.new, 1);
        (after, start, start + input.new.len())
    } else {
        (input.new.clone(), 0, input.new.len())
    };
    (before.len() + after.len() <= MAX_REVIEW_BYTES).then_some((after, start, end))
}
impl ConduitApp {
    pub(in crate::app) fn file_proposal_event(
        &mut self,
        session: u64,
        key: &str,
        root: &Path,
        update: Update,
        cx: &mut Context<Self>,
    ) {
        let visible: HashSet<_> = self.visible_file_documents().into_iter().collect();
        for (&id, document) in &mut self.files.documents {
            if matches!(update, Update::Started | Update::Content(_))
                && (self.selected_session != Some(session) || !visible.contains(&id))
            {
                continue;
            }
            match &update {
                Update::Started => {
                    if document.proposals.attempts.len() >= 8
                        || document
                            .proposals
                            .attempts
                            .iter()
                            .any(|a| a.session == session && a.key == key)
                    {
                        for a in &mut document.proposals.attempts {
                            a.blocked = true;
                        }
                        publish(document, cx);
                        continue;
                    }
                    if document.stamp.is_some()
                        && document.error.is_none()
                        && !document.loading
                        && !document.is_image
                        && document.text.len() <= MAX_REVIEW_BYTES / 2
                        && document.proposals.attempts.len() < 8
                    {
                        document.proposals.attempts.push(Attempt {
                            session,
                            key: key.into(),
                            before: document.text.clone(),
                            stamp: document.stamp.clone().unwrap(),
                            started: Instant::now(),
                            blocked: false,
                            snapshot: None,
                        });
                    }
                }
                Update::Content(input) => {
                    if let Some(attempt) = document
                        .proposals
                        .attempts
                        .iter_mut()
                        .find(|a| a.session == session && a.key == key && !a.blocked)
                    {
                        if document.stamp.as_ref() != Some(&attempt.stamp)
                            || document.text != attempt.before
                            || document.error.is_some()
                        {
                            attempt.blocked = true;
                        } else if let Some((text, start, end)) =
                            proposed(input, root, &document.key, &attempt.before)
                        {
                            attempt.snapshot = Some(Snapshot {
                                session,
                                key: key.into(),
                                text: text.into(),
                                range: start..end,
                                complete: input.complete,
                            });
                        } else {
                            attempt.blocked = true;
                        }
                    }
                    if document
                        .proposals
                        .attempts
                        .iter()
                        .filter(|a| a.snapshot.is_some())
                        .count()
                        > 1
                    {
                        for attempt in &mut document.proposals.attempts {
                            attempt.blocked = true;
                        }
                    }
                }
                Update::Ended => document.proposals.end(session, Some(key)),
                Update::Reset => document.proposals.end(session, None),
            }
            publish(document, cx);
        }
    }
    pub(in crate::app) fn end_file_proposals(
        &mut self,
        session: u64,
        key: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        for document in self.files.documents.values_mut() {
            document.proposals.end(session, key);
            publish(document, cx);
        }
    }
    pub(in crate::app) fn maintain_file_proposals(&mut self, cx: &mut Context<Self>) {
        let visible: HashSet<_> = self.visible_file_documents().into_iter().collect();
        for (&id, document) in &mut self.files.documents {
            document.proposals.attempts.retain(|a| {
                self.selected_session == Some(a.session)
                    && visible.contains(&id)
                    && a.started.elapsed() < Duration::from_secs(120)
                    && self
                        .session_control
                        .runs
                        .get(&a.session)
                        .is_some_and(|run| !run.runtime.is_cancelled())
                    && document.stamp.as_ref() == Some(&a.stamp)
                    && document.text == a.before
                    && document.error.is_none()
            });
            publish(document, cx);
        }
    }
}
pub(super) fn invalidate(document: &mut Document, cx: &mut Context<ConduitApp>) {
    document.proposals.clear();
    publish(document, cx);
}

pub(super) fn conflicting_tool(
    document: &mut Document,
    session: u64,
    item: &ChatItem,
    cx: &mut Context<ConduitApp>,
) {
    for attempt in &mut document.proposals.attempts {
        if attempt.session != session || attempt.key != item.key {
            attempt.blocked = true;
        }
    }
    publish(document, cx);
}

fn publish(document: &mut Document, cx: &mut Context<ConduitApp>) {
    let snapshot = document.proposals.current();
    if document.preview.read(cx).has_proposal() != snapshot.is_some() {
        document.revision += 1;
        cx.notify();
    }
    document
        .preview
        .update(cx, |preview, cx| preview.set_proposal(snapshot, cx));
}

#[cfg(test)]
mod tests {
    use super::{FileKey, Input, MAX_REVIEW_BYTES, proposed};
    use std::path::Path;
    #[test]
    fn content_proposal_requires_exact_unique_baseline_and_matching_path() {
        let root = Path::new("C:/workspace");
        let key = FileKey {
            workspace: root.into(),
            relative: "note.txt".into(),
        };
        let mut input = Input {
            path: "nested/../note.txt".into(),
            old: Some("old".into()),
            new: "new".into(),
            complete: false,
        };
        assert_eq!(
            proposed(&input, root, &key, "prefix old suffix"),
            Some(("prefix new suffix".into(), 7, 10))
        );
        assert!(proposed(&input, root, &key, "old old").is_none());
        assert!(proposed(&input, root, &key, "missing").is_none());
        input.path = "../note.txt".into();
        assert!(proposed(&input, root, &key, "old").is_none());
        input.path = "note.txt".into();
        input.old = Some(String::new());
        assert!(proposed(&input, root, &key, "old").is_none());
        input.old = None;
        assert_eq!(proposed(&input, root, &key, ""), Some(("new".into(), 0, 3)));
        input.new = "x".repeat(MAX_REVIEW_BYTES + 1);
        assert!(proposed(&input, root, &key, "").is_none());
    }
}
