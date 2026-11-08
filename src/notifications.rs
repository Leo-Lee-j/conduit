//! Application notifications, independent of GPUI and of the conversation store.
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};

pub(crate) mod history;

pub(crate) const MAX_RECORDS: usize = 200;
const RETENTION_SECONDS: u64 = 30 * 24 * 60 * 60;
const MAX_TEXT_BYTES: usize = 4096;
const MAX_SEEN_EVENTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Level {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Source {
    App,
    Session,
    Git,
    Update,
    Settings,
    Persistence,
}

impl Source {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::App => conduit::i18n::text_static("settings.appearance.application"),
            Self::Session => conduit::i18n::text_static("notifications.session"),
            Self::Git => "Git",
            Self::Update => conduit::i18n::text_static("settings.general.application_updates"),
            Self::Settings => conduit::i18n::text_static("files.settings"),
            Self::Persistence => conduit::i18n::text_static("notifications.data_saving"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    Session {
        workspace: PathBuf,
        session: u64,
        created_at: u64,
        native_id: Option<String>,
    },
    Workspace(PathBuf),
    Settings,
    Updates,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Lifecycle {
    Recorded,
    Active,
    Resolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presentation {
    History,
    Toast,
    Banner,
}

/// The only input business workflows need. Targets describe navigation, never commands.
#[derive(Clone, Debug)]
pub(crate) struct Notice {
    pub source: Source,
    pub level: Level,
    pub title: String,
    pub message: String,
    pub message_i18n: Option<conduit::i18n::Message>,
    pub target: Option<Target>,
    pub key: Option<String>,
    pub event: Option<String>,
    pub active: bool,
    pub presentation: Presentation,
}

impl Notice {
    pub(crate) fn new(
        source: Source,
        level: Level,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            level,
            title: title.into(),
            message: message.into(),
            message_i18n: None,
            target: None,
            key: None,
            event: None,
            active: false,
            presentation: Presentation::Toast,
        }
    }
    pub(crate) fn localized_message(mut self, message: conduit::i18n::Message) -> Self {
        self.message = message.render();
        self.message_i18n = Some(message);
        self
    }
    pub(crate) fn target(mut self, target: Target) -> Self {
        self.target = Some(target);
        self
    }
    pub(crate) fn key(mut self, key: impl Into<String>, event: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self.event = Some(event.into());
        self
    }
    pub(crate) fn active(mut self) -> Self {
        self.active = true;
        self
    }
    pub(crate) fn presentation(mut self, value: Presentation) -> Self {
        self.presentation = value;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ApprovalOutcome {
    Allowed,
    Denied,
    Cancelled,
    Expired,
}
impl ApprovalOutcome {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Allowed => conduit::i18n::text_static("stream_events.allowed"),
            Self::Denied => conduit::i18n::text_static("stream_events.denied"),
            Self::Cancelled => conduit::i18n::text_static("events.cancelled"),
            Self::Expired => conduit::i18n::text_static("notifications.expired"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Record {
    pub id: u64,
    pub source: Source,
    pub level: Level,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_i18n: Option<conduit::i18n::Message>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_i18n: Option<conduit::i18n::Message>,
    pub target: Option<Target>,
    pub created_at: u64,
    pub updated_at: u64,
    pub read_at: Option<u64>,
    pub lifecycle: Lifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_outcome: Option<ApprovalOutcome>,
    pub occurrences: u32,
    pub key: Option<String>,
    pub seen_events: Vec<String>,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub schema: u32,
    pub next_id: u64,
    pub records: Vec<Record>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            schema: 1,
            next_id: 1,
            records: vec![],
        }
    }
}

#[derive(Default)]
pub(crate) struct Center {
    snapshot: Snapshot,
    revision: u64,
}

#[derive(Debug)]
pub(crate) struct Published {
    pub id: u64,
    pub changed: bool,
    pub presentation: Presentation,
}

fn truncate(text: &mut String, limit: usize) {
    if text.len() > limit {
        let mut end = limit.saturating_sub('…'.len_utf8());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
}

impl Center {
    pub(crate) fn restore(mut snapshot: Snapshot, now: u64) -> Result<Self, String> {
        if snapshot.schema != 1 {
            return Err(conduit::i18n::text_static(
                "notifications.unsupported_notification_history_version_original_file_preserved",
            )
            .into());
        }
        let mut ids = HashSet::new();
        for record in &mut snapshot.records {
            if record.id == 0
                || record.id == u64::MAX
                || !ids.insert(record.id)
                || record.seen_events.len() > MAX_SEEN_EVENTS
                || record.key.as_ref().is_some_and(|key| key.len() > 4096)
                || record.seen_events.iter().any(|event| event.len() > 4096)
            {
                return Err(conduit::i18n::text_static(
                    "notifications.invalid_notification_history_record_original_file_preserved",
                )
                .into());
            }
            if let Some(
                Target::Workspace(path)
                | Target::Session {
                    workspace: path, ..
                },
            ) = &record.target
                && !path.is_absolute()
            {
                return Err(conduit::i18n::text_static(
                    "notifications.invalid_notification_history_target_original_file_preserved",
                )
                .into());
            }
            truncate(&mut record.title, 256);
            truncate(&mut record.message, MAX_TEXT_BYTES - record.title.len());
            if record.lifecycle == Lifecycle::Active {
                record.lifecycle = Lifecycle::Resolved;
                record.message_i18n = None;
                record.message.push_str(conduit::i18n::text_static(
                    "notifications.the_original_task_has_ended_check_its",
                ));
                truncate(&mut record.message, MAX_TEXT_BYTES - record.title.len());
            }
        }
        snapshot.next_id = snapshot.next_id.max(
            snapshot
                .records
                .iter()
                .map(|record| record.id + 1)
                .max()
                .unwrap_or(1),
        );
        if snapshot.next_id == u64::MAX {
            return Err(conduit::i18n::text_static(
                "notifications.invalid_notification_history_id_original_file_preserved",
            )
            .into());
        }
        snapshot
            .records
            .sort_by_key(|record| std::cmp::Reverse((record.updated_at, record.id)));
        let mut result = Self {
            snapshot,
            revision: 1,
        };
        result.prune(now);
        Ok(result)
    }
    pub(crate) fn records(&self) -> &[Record] {
        &self.snapshot.records
    }
    pub(crate) fn get(&self, id: u64) -> Option<&Record> {
        self.snapshot.records.iter().find(|record| record.id == id)
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }
    pub(crate) fn unread(&self) -> usize {
        self.records()
            .iter()
            .filter(|record| record.read_at.is_none())
            .count()
    }

    pub(crate) fn publish(&mut self, mut request: Notice, now: u64) -> Published {
        truncate(&mut request.title, 256);
        truncate(&mut request.message, MAX_TEXT_BYTES - request.title.len());
        if let (Some(key), Some(event)) = (&request.key, &request.event)
            && let Some(record) = self.records().iter().find(|record| {
                record.source == request.source
                    && record.key.as_ref() == Some(key)
                    && record.target == request.target
                    && record.seen_events.contains(event)
            })
        {
            return Published {
                id: record.id,
                changed: false,
                presentation: request.presentation,
            };
        }
        // The caller binds keys to a task/run, not to a currently selected UI context.
        let existing = request.key.as_ref().and_then(|key| {
            self.snapshot.records.iter().position(|record| {
                record.source == request.source
                    && record.key.as_ref() == Some(key)
                    && record.target == request.target
            })
        });
        if let Some(index) = existing {
            let record = &self.snapshot.records[index];
            if request
                .event
                .as_ref()
                .is_some_and(|event| record.seen_events.contains(event))
            {
                return Published {
                    id: record.id,
                    changed: false,
                    presentation: request.presentation,
                };
            }
        }
        let existing =
            existing.filter(|index| self.snapshot.records[*index].lifecycle != Lifecycle::Resolved);
        self.revision += 1;
        let mut record = if let Some(index) = existing {
            let mut record = self.snapshot.records.remove(index);
            record.occurrences = if record.level == request.level
                && record.title == request.title
                && record.message == request.message
            {
                record.occurrences.saturating_add(1)
            } else {
                1
            };
            record
        } else {
            let id = self.snapshot.next_id;
            self.snapshot.next_id += 1;
            Record {
                id,
                approval_outcome: None,
                source: request.source,
                level: request.level,
                title: String::new(),
                title_i18n: None,
                message_i18n: None,
                message: String::new(),
                target: request.target.clone(),
                created_at: now,
                updated_at: now,
                read_at: None,
                lifecycle: Lifecycle::Recorded,
                occurrences: 1,
                key: request.key.clone(),
                seen_events: vec![],
                revision: 0,
            }
        };
        record.level = request.level;
        record.title_i18n = conduit::i18n::Message::capture_label(&request.title);
        record.message_i18n = request.message_i18n;
        record.title = request.title;
        record.message = request.message;
        record.updated_at = now;
        record.read_at = None;
        record.lifecycle = if request.active {
            Lifecycle::Active
        } else {
            Lifecycle::Recorded
        };
        record.revision = self.revision;
        if let Some(event) = request.event {
            record.seen_events.push(event);
            if record.seen_events.len() > MAX_SEEN_EVENTS {
                record.seen_events.remove(0);
            }
        }
        let id = record.id;
        self.snapshot.records.insert(0, record);
        self.prune(now);
        Published {
            id,
            changed: true,
            presentation: request.presentation,
        }
    }

    pub(crate) fn mark_read(&mut self, id: u64, now: u64) {
        if let Some(record) = self
            .snapshot
            .records
            .iter_mut()
            .find(|record| record.id == id)
            && record.read_at.is_none()
        {
            record.read_at = Some(now);
            self.revision += 1;
        }
    }
    pub(crate) fn mark_all_read(&mut self, now: u64) {
        for record in &mut self.snapshot.records {
            if record.read_at.is_none() {
                record.read_at = Some(now);
                self.revision += 1;
            }
        }
    }
    pub(crate) fn clear_read(&mut self) {
        let before = self.snapshot.records.len();
        self.snapshot
            .records
            .retain(|record| record.read_at.is_none() || record.lifecycle == Lifecycle::Active);
        if self.snapshot.records.len() != before {
            self.revision += 1;
        }
    }
    pub(crate) fn remove_read(&mut self, id: u64) {
        if let Some(index) = self.snapshot.records.iter().position(|record| {
            record.id == id && record.read_at.is_some() && record.lifecycle != Lifecycle::Active
        }) {
            self.snapshot.records.remove(index);
            self.revision += 1;
        }
    }
    pub(crate) fn resolve(&mut self, key: &str, now: u64) {
        for record in &mut self.snapshot.records {
            if record.key.as_deref() == Some(key) && record.lifecycle == Lifecycle::Active {
                record.lifecycle = Lifecycle::Resolved;
                record.updated_at = now;
                self.revision += 1;
                record.revision = self.revision;
            }
        }
    }
    pub(crate) fn resolve_session(&mut self, session: u64, now: u64) {
        let keys: Vec<_> = self.records().iter().filter(|record| {
            matches!(&record.target, Some(Target::Session { session: id, .. }) if *id == session)
                && record.lifecycle == Lifecycle::Active
        }).filter_map(|record| record.key.clone()).collect();
        for key in keys {
            self.resolve(&key, now);
        }
    }
    /// Updates the original record only after a live approval ends; never republishes it.
    pub(crate) fn resolve_approval(&mut self, id: u64, outcome: ApprovalOutcome, now: u64) {
        if let Some(record) = self
            .snapshot
            .records
            .iter_mut()
            .find(|record| record.id == id && record.lifecycle == Lifecycle::Active)
        {
            record.lifecycle = Lifecycle::Resolved;
            record.approval_outcome = Some(outcome);
            record.updated_at = now;
            if matches!(outcome, ApprovalOutcome::Allowed | ApprovalOutcome::Denied) {
                record.read_at.get_or_insert(now);
            }
            self.revision += 1;
            record.revision = self.revision;
        }
    }
    pub(crate) fn prune(&mut self, now: u64) {
        let before = self.snapshot.records.len();
        self.snapshot
            .records
            .retain(|record| now.saturating_sub(record.updated_at) <= RETENTION_SECONDS);
        while self.snapshot.records.len() > MAX_RECORDS {
            let index = self
                .snapshot
                .records
                .iter()
                .rposition(|record| {
                    record.read_at.is_some() && record.lifecycle != Lifecycle::Active
                })
                .unwrap_or(self.snapshot.records.len() - 1);
            self.snapshot.records.remove(index);
        }
        if before != self.snapshot.records.len() {
            self.revision += 1;
        }
    }
}

impl Record {
    pub(crate) fn display_title(&self) -> String {
        self.title_i18n
            .as_ref()
            .map(conduit::i18n::Message::render)
            .unwrap_or_else(|| conduit::i18n::retained(&self.title).to_string())
    }
    pub(crate) fn display_message(&self) -> String {
        self.message_i18n
            .as_ref()
            .map(conduit::i18n::Message::render)
            .unwrap_or_else(|| self.message.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notice(event: &str) -> Notice {
        Notice::new(Source::Git, Level::Error, "推送失败", "请检查连接")
            .key("operation-1", event)
            .target(Target::Workspace(std::env::temp_dir().join("repo-a")))
    }
    #[test]
    fn idempotence_preserves_reads_and_does_not_rewind_newer_events() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        let first = center.publish(notice("failed"), 100);
        center.mark_read(first.id, 101);
        assert!(!center.publish(notice("failed"), 102).changed);
        assert_eq!(center.unread(), 0);
        assert_eq!(center.publish(notice("retry-failed"), 103).id, first.id);
        assert!(!center.publish(notice("failed"), 104).changed);
        assert_eq!(center.records()[0].occurrences, 2);
        assert_eq!(center.unread(), 1);
        let other = notice("failed").target(Target::Workspace(std::env::temp_dir().join("repo-b")));
        assert_ne!(center.publish(other, 105).id, first.id);
    }
    #[test]
    fn reading_and_clearing_cannot_resolve_an_active_problem() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        let first = center.publish(notice("failed").active(), 100);
        center.mark_all_read(101);
        center.clear_read();
        assert_eq!(center.get(first.id).unwrap().lifecycle, Lifecycle::Active);
        center.resolve("operation-1", 102);
        center.clear_read();
        assert!(center.records().is_empty());
        assert!(center.publish(notice("failed").active(), 103).changed);
    }
    #[test]
    fn resolved_events_do_not_reopen_on_late_delivery_but_new_episodes_can_start() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        let first = center.publish(notice("failure-1").active(), 100).id;
        center.resolve("operation-1", 101);
        assert!(!center.publish(notice("failure-1").active(), 102).changed);
        assert_eq!(center.get(first).unwrap().lifecycle, Lifecycle::Resolved);
        let next = center.publish(notice("failure-2").active(), 103).id;
        assert_ne!(next, first);
        assert!(!center.publish(notice("failure-1").active(), 104).changed);
        center.remove_read(next);
        assert!(center.get(next).is_some());
    }
    #[test]
    fn restored_history_is_bounded_and_pending_actions_end() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        for index in 0..205 {
            center.publish(
                notice("created")
                    .key(format!("task-{index}"), "created")
                    .active(),
                100 + index,
            );
        }
        assert_eq!(center.records().len(), MAX_RECORDS);
        let copy: Snapshot =
            serde_json::from_slice(&serde_json::to_vec(&center.snapshot()).unwrap()).unwrap();
        let restored = Center::restore(copy, 400).unwrap();
        assert_eq!(restored.unread(), MAX_RECORDS);
        assert!(
            restored
                .records()
                .iter()
                .all(|record| record.lifecycle == Lifecycle::Resolved)
        );
        assert!(
            Center::restore(restored.snapshot(), RETENTION_SECONDS + 500)
                .unwrap()
                .records()
                .is_empty()
        );
    }
    #[test]
    fn text_limits_preserve_utf8_and_navigation_identity() {
        let _locale = crate::test_support::chinese();
        let mut request = notice("created");
        request.title = "提示".repeat(300);
        request.message = "内容".repeat(5000);
        let mut center = Center::default();
        center.publish(request.clone(), 100);
        let record = &center.records()[0];
        assert!(record.title.len() + record.message.len() <= MAX_TEXT_BYTES);
        assert_eq!(record.target, request.target);
    }

    #[test]
    fn confirmed_approval_outcome_is_read_terminal_and_persisted() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        let id = center.publish(notice("waiting").active(), 100).id;
        center.resolve_approval(id, ApprovalOutcome::Denied, 101);
        let revision = center.revision();
        center.resolve_approval(id, ApprovalOutcome::Allowed, 102);
        assert_eq!(center.revision(), revision);
        let record = center.get(id).unwrap();
        assert_eq!(record.approval_outcome, Some(ApprovalOutcome::Denied));
        assert_eq!(record.read_at, Some(101));
        assert_eq!(record.occurrences, 1);
        assert!(!center.publish(notice("waiting").active(), 103).changed);
        let snapshot =
            serde_json::from_slice(&serde_json::to_vec(&center.snapshot()).unwrap()).unwrap();
        let restored = Center::restore(snapshot, 104).unwrap();
        assert_eq!(
            restored.get(id).unwrap().approval_outcome,
            Some(ApprovalOutcome::Denied)
        );
        assert_eq!(restored.unread(), 0);
    }

    #[test]
    fn expired_approvals_remain_unread_and_old_history_needs_no_outcome_field() {
        let _locale = crate::test_support::chinese();
        let mut center = Center::default();
        let id = center.publish(notice("waiting").active(), 100).id;
        let legacy = serde_json::to_value(center.snapshot()).unwrap();
        assert!(legacy["records"][0].get("approval_outcome").is_none());
        let restored = Center::restore(serde_json::from_value(legacy).unwrap(), 101).unwrap();
        assert_eq!(restored.get(id).unwrap().lifecycle, Lifecycle::Resolved);
        center.resolve_approval(id, ApprovalOutcome::Expired, 101);
        assert_eq!(center.unread(), 1);
        assert_eq!(
            center.get(id).unwrap().approval_outcome,
            Some(ApprovalOutcome::Expired)
        );
    }
}

#[cfg(test)]
mod localization_tests {
    use super::*;
    #[test]
    fn history_translates_structured_messages_but_preserves_user_text() {
        let _locale = crate::test_support::scoped("zh-CN");
        let mut center = Center::default();
        let user = center.publish(
            Notice::new(
                Source::Session,
                Level::Success,
                conduit::i18n::text("stream_events.session_completed"),
                "已完成",
            ),
            10,
        );
        let update = center.publish(
            Notice::new(
                Source::Update,
                Level::Info,
                conduit::i18n::text("update.new_version_available_010474"),
                "",
            )
            .localized_message(conduit::i18n::Message::new(
                "update.new_version_available",
                &[("arg0", "2.0.0".into())],
            )),
            11,
        );
        let encoded = serde_json::to_string(&center.snapshot).unwrap();
        conduit::i18n::set_locale(&conduit::i18n::Locale::default());
        let restored = Center::restore(serde_json::from_str(&encoded).unwrap(), 12).unwrap();
        assert_eq!(
            restored.get(user.id).unwrap().display_title(),
            "Session completed"
        );
        assert_eq!(restored.get(user.id).unwrap().display_message(), "已完成");
        assert_eq!(
            restored.get(update.id).unwrap().display_message(),
            "New version available: 2.0.0"
        );
        let mut legacy: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        for record in legacy["records"].as_array_mut().unwrap() {
            record.as_object_mut().unwrap().remove("title_i18n");
            record.as_object_mut().unwrap().remove("message_i18n");
        }
        assert!(Center::restore(serde_json::from_value(legacy).unwrap(), 12).is_ok());
    }
}
