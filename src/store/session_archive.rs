use crate::domain::SessionRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoArchiveDays {
    #[serde(rename = "3")]
    Three,
    #[default]
    #[serde(rename = "7")]
    Seven,
    #[serde(rename = "15")]
    Fifteen,
    #[serde(rename = "30")]
    Thirty,
}

impl AutoArchiveDays {
    pub const ALL: [Self; 4] = [Self::Three, Self::Seven, Self::Fifteen, Self::Thirty];

    pub const fn days(self) -> u64 {
        match self {
            Self::Three => 3,
            Self::Seven => 7,
            Self::Fifteen => 15,
            Self::Thirty => 30,
        }
    }

    pub fn label(self) -> String {
        conduit::i18n::format(
            "settings.sessions.archive_days",
            &[("days", self.days().to_string())],
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoArchiveSettings {
    pub enabled: bool,
    pub after: AutoArchiveDays,
    /// Local calendar day, persisted so restarting cannot repeat the daily sweep.
    pub last_check_day: Option<i32>,
    /// Restoring an archived session gives it a full new inactivity period.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub restored_at: HashMap<u64, u64>,
}

impl AutoArchiveSettings {
    pub fn due(&self, day: i32) -> bool {
        self.enabled && self.last_check_day.is_none_or(|last| last < day)
    }

    pub fn eligible(&self, session: &SessionRecord, now: u64) -> bool {
        let activity = session.last_activity_at().max(
            self.restored_at
                .get(&session.id)
                .copied()
                .unwrap_or_default(),
        );
        !session.archived
            && !session.is_subagent
            && !session.pinned
            && activity > 0
            && now.saturating_sub(activity) >= self.after.days() * 24 * 60 * 60
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> SessionRecord {
        serde_json::from_value(serde_json::json!({
            "id": 1, "provider": "Codex", "created_at": 1000, "exit_code": null
        }))
        .unwrap()
    }

    #[test]
    fn auto_archive_uses_last_activity_and_each_selected_period() {
        let mut session = session();
        session.messages.push(
            serde_json::from_value(serde_json::json!({
                "key": "reply", "kind": "Assistant", "text": "hello", "created_at": 2000
            }))
            .unwrap(),
        );
        for after in AutoArchiveDays::ALL {
            let settings = AutoArchiveSettings {
                after,
                ..Default::default()
            };
            let boundary = 2000 + after.days() * 24 * 60 * 60;
            assert!(!settings.eligible(&session, boundary - 1));
            assert!(settings.eligible(&session, boundary));
            assert!(!settings.eligible(&session, 0));
            let mut protected = session.clone();
            protected.pinned = true;
            assert!(!settings.eligible(&protected, boundary));
            protected.pinned = false;
            protected.archived = true;
            assert!(!settings.eligible(&protected, boundary));
            protected.archived = false;
            protected.is_subagent = true;
            assert!(!settings.eligible(&protected, boundary));
        }
    }

    #[test]
    fn auto_archive_daily_check_and_restore_grace_survive_serialization() {
        let old = serde_json::from_str::<AutoArchiveSettings>("{}").unwrap();
        assert!(!old.enabled);
        assert_eq!(old.after, AutoArchiveDays::Seven);
        assert!(!old.due(100));
        let settings = AutoArchiveSettings {
            enabled: true,
            last_check_day: Some(100),
            restored_at: [(1, 900_000)].into(),
            ..Default::default()
        };
        let restored: AutoArchiveSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(!restored.due(100));
        assert!(!restored.due(99));
        assert!(restored.due(101));
        assert!(!restored.eligible(&session(), 900_000));
        assert!(restored.eligible(&session(), 900_000 + 7 * 24 * 60 * 60));
        assert!(serde_json::from_str::<AutoArchiveDays>("\"2\"").is_err());
    }

    #[test]
    fn auto_archive_can_check_unloaded_transcripts_without_loading_history() {
        let settings = AutoArchiveSettings::default();
        let mut record = session();
        record.transcript.loaded = false;
        record.transcript.reference = Some(
            serde_json::from_value(serde_json::json!({
                "hash": "a".repeat(64),
                "message_count": 2000,
                "last_activity_at": 3000,
                "has_user": true,
                "bytes": 10000,
                "attachments": []
            }))
            .unwrap(),
        );
        let boundary = 3000 + settings.after.days() * 24 * 60 * 60;
        assert!(!settings.eligible(&record, boundary - 1));
        assert!(settings.eligible(&record, boundary));
        assert!(record.messages.is_empty());
        record.transcript.reference = None;
        assert!(settings.eligible(&record, boundary - 1));
        record.created_at = 0;
        assert!(!settings.eligible(&record, boundary));
    }
}
