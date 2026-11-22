//! Evidence joins are conservative: a tool result must reproduce a retained
//! pre-event snapshot exactly. The UI says "matches", never claims exclusive authorship.
use super::super::file_preview::changes::MAX_REVIEW_BYTES;
use super::*;
use crate::message_status::ToolStatus;

pub(super) struct Observation {
    session: u64,
    key: String,
    before: SharedString,
    expected: String,
    completed: bool,
    ambiguous: bool,
    started: Instant,
}

impl ConduitApp {
    pub(in crate::app) fn observe_file_tool(
        &mut self,
        session: u64,
        root: &Path,
        item: &ChatItem,
        cx: &mut Context<Self>,
    ) {
        if item.kind != ItemKind::Tool {
            return;
        }
        for document in self.files.documents.values_mut() {
            if !item.input.is_empty()
                && expected_for(item, root, &document.key, &document.text).is_some()
            {
                proposals::conflicting_tool(document, session, item, cx);
            }
            document
                .observations
                .retain(|o| o.started.elapsed() < Duration::from_secs(120));
            if let Some(observation) = document
                .observations
                .iter_mut()
                .find(|o| o.session == session && o.key == item.key)
            {
                if item.status == ToolStatus::Completed {
                    observation.completed = true;
                } else if !matches!(item.status, ToolStatus::Running | ToolStatus::Output) {
                    observation.ambiguous = true;
                }
                // An authoritative final input must still describe the same transformation.
                if !item.input.is_empty()
                    && expected_for(item, root, &document.key, &observation.before)
                        .is_none_or(|expected| expected != observation.expected)
                {
                    observation.ambiguous = true;
                }
            } else if matches!(item.status, ToolStatus::Running | ToolStatus::Output)
                && document.stamp.is_some()
                && document.error.is_none()
                && !document.is_image
                && document.observations.len() < 8
                && let Some(expected) = expected_for(item, root, &document.key, &document.text)
            {
                document
                    .observations
                    .retain(|o| !o.completed || o.before == document.text);
                let ambiguous = !document.observations.is_empty();
                for observation in &mut document.observations {
                    observation.ambiguous = true;
                }
                document.observations.push(Observation {
                    session,
                    key: item.key.clone(),
                    before: document.text.clone(),
                    expected,
                    completed: false,
                    ambiguous,
                    started: Instant::now(),
                });
            }
            match_observation(document);
        }
        cx.notify();
    }
}

pub(super) fn match_observation(document: &mut Document) {
    let Some(changes) = &document.changes else {
        document.matched_tool = None;
        return;
    };
    let matching: Vec<_> = document
        .observations
        .iter()
        .filter(|o| {
            o.completed
                && !o.ambiguous
                && o.started.elapsed() < Duration::from_secs(120)
                && o.before.as_ref() == changes.before.as_ref()
                && o.expected == document.text.as_ref()
                && o.expected == changes.after.as_ref()
        })
        .collect();
    let matched = (matching.len() == 1).then(|| (matching[0].session, matching[0].key.clone()));
    if document.matched_tool != matched {
        document.matched_tool = matched;
        document.revision += 1;
    }
}

fn expected_for(item: &ChatItem, root: &Path, key: &FileKey, before: &str) -> Option<String> {
    if before.len() > MAX_REVIEW_BYTES || item.input.len() + item.detail.len() > MAX_REVIEW_BYTES {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&item.input).ok()?;
    let target = normalized(&key.workspace.join(&key.relative))?;
    let matches = |value: &serde_json::Value| {
        let path = super::super::tools::json_path(value)?;
        let path = Path::new(&path);
        (normalized(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        })? == target)
            .then_some(())
    };
    let expected = match item.text.trim() {
        "Write" => {
            matches(&value)?;
            value.get("content")?.as_str()?.to_string()
        }
        "Edit" => {
            matches(&value)?;
            replace(before, &value)?
        }
        "MultiEdit" => {
            matches(&value)?;
            let mut result = before.to_string();
            for edit in value.get("edits")?.as_array()? {
                result = replace(&result, edit)?;
            }
            result
        }
        _ => {
            let changes = value.as_array()?;
            let matching: Vec<_> = changes
                .iter()
                .filter(|change| matches(change).is_some())
                .collect();
            if matching.len() != 1 {
                return None;
            }
            apply_patch(before, matching[0].get("diff")?.as_str()?)?
        }
    };
    (expected.len() + before.len() <= MAX_REVIEW_BYTES && expected != before).then_some(expected)
}

pub(super) fn normalized(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut result = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => (),
            Component::ParentDir => {
                if !result.pop() {
                    return None;
                }
            }
            _ => result.push(c.as_os_str()),
        }
    }
    Some(result)
}

fn replace(before: &str, edit: &serde_json::Value) -> Option<String> {
    let old = edit.get("old_string")?.as_str()?;
    let new = edit.get("new_string")?.as_str()?;
    if old.is_empty() {
        return None;
    }
    let count = before.matches(old).count();
    if count == 0
        || (count != 1
            && !edit
                .get("replace_all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false))
    {
        return None;
    }
    let size = before
        .len()
        .checked_sub(count * old.len())?
        .checked_add(count.checked_mul(new.len())?)?;
    if size > MAX_REVIEW_BYTES {
        return None;
    }
    Some(before.replace(old, new))
}

/// Strict unified hunks only: full context, counts, order and new offsets are
/// validated. No fuzzy matching or synthetic hunk locations from tool cards.
fn apply_patch(before: &str, patch: &str) -> Option<String> {
    let lines: Vec<_> = before.split_inclusive('\n').collect();
    let raw: Vec<_> = patch.split_inclusive('\n').collect();
    let (mut pos, mut at, mut written, mut hunks) = (0, 0, 0, 0);
    let mut result = String::new();
    while pos < raw.len() {
        if !raw[pos].starts_with("@@ ") {
            if hunks > 0 {
                return None;
            }
            pos += 1;
            continue;
        }
        let header: Vec<_> = raw[pos].split_whitespace().collect();
        if header.len() < 4 || header[3] != "@@" {
            return None;
        }
        let range = |token: &str, sign: char| -> Option<(usize, usize)> {
            let token = token.strip_prefix(sign)?;
            let (start, count) = token.split_once(',').unwrap_or((token, "1"));
            Some((start.parse().ok()?, count.parse().ok()?))
        };
        let (old_start, old_count) = range(header[1], '-')?;
        let (new_start, new_count) = range(header[2], '+')?;
        let target = if old_count == 0 {
            old_start
        } else {
            old_start.checked_sub(1)?
        };
        if target < at || target > lines.len() {
            return None;
        }
        for line in &lines[at..target] {
            result.push_str(line);
            written += 1;
        }
        at = target;
        if written
            != if new_count == 0 {
                new_start
            } else {
                new_start.checked_sub(1)?
            }
        {
            return None;
        }
        pos += 1;
        let (mut consumed, mut added) = (0, 0);
        while pos < raw.len() && !raw[pos].starts_with("@@ ") {
            let kind = *raw[pos].as_bytes().first()?;
            if !matches!(kind, b' ' | b'+' | b'-') {
                return None;
            }
            let mut text = &raw[pos][1..];
            pos += 1;
            if pos < raw.len() && raw[pos].starts_with("\\ No newline at end of file") {
                text = text.strip_suffix('\n').unwrap_or(text);
                pos += 1;
            }
            if kind != b'+' {
                if lines.get(at).copied() != Some(text) {
                    return None;
                }
                consumed += 1;
                at += 1;
            }
            if kind != b'-' {
                result.push_str(text);
                added += 1;
                written += 1;
            }
        }
        if consumed != old_count || added != new_count {
            return None;
        }
        hunks += 1;
    }
    if hunks == 0 {
        return None;
    }
    for line in &lines[at..] {
        result.push_str(line);
    }
    (result.len() <= MAX_REVIEW_BYTES).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::{apply_patch, replace};
    use serde_json::json;
    #[test]
    fn content_review_tool_proof_rejects_ambiguous_replacements_and_incomplete_patches() {
        assert!(replace("x x", &json!({"old_string":"x","new_string":"y"})).is_none());
        assert_eq!(
            replace(
                "x x",
                &json!({"old_string":"x","new_string":"y","replace_all":true})
            )
            .unwrap(),
            "y y"
        );
        let patch = "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        assert_eq!(apply_patch("a\nb\nc\n", patch).unwrap(), "a\nB\nc\n");
        assert!(apply_patch("a\nexternal\nc\n", patch).is_none());
        assert!(apply_patch("a\nb\nc\n", &patch[..patch.len() - 3]).is_none());
        assert!(apply_patch("a\nb\nc\n", &patch.replace("+1,3", "+10,3")).is_none());
        assert_eq!(apply_patch("old","@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n").unwrap(),"new");
    }
}
