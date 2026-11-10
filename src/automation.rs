//! App-local schedules. Persist a claim before invoking an engine; never catch up silently.
use crate::workflow::Preset;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Trigger {
    Interval { seconds: u64, next_at: u64 },
    SessionCompleted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Schedule {
    pub id: u64,
    pub preset: Preset,
    pub trigger: Trigger,
    pub enabled: bool,
    pub last_event: Option<String>,
    pub last_claim: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Status {
    Claimed,
    Running,
    Complete,
    Failed,
    Cancelled,
    Missed,
    Interrupted,
}
impl Status {
    pub fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Claimed => "automation.claimed",
            Self::Running => "automation.running",
            Self::Complete => "automation.complete",
            Self::Failed => "automation.failed",
            Self::Cancelled => "automation.cancelled",
            Self::Missed => "automation.missed",
            Self::Interrupted => "automation.interrupted",
        })
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Claimed | Self::Running)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Run {
    pub id: u64,
    pub schedule: u64,
    #[serde(default)]
    pub workspace: Option<std::path::PathBuf>,
    pub at: u64,
    pub session: Option<u64>,
    pub status: Status,
    pub detail: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Scheduler {
    pub schedules: Vec<Schedule>,
    pub runs: Vec<Run>,
    next_id: u64,
}

impl Scheduler {
    fn allocate(&mut self) -> Option<u64> {
        self.next_id = self
            .schedules
            .iter()
            .map(|s| s.id)
            .chain(self.runs.iter().map(|r| r.id))
            .max()
            .unwrap_or(0)
            .max(self.next_id)
            .checked_add(1)?;
        Some(self.next_id)
    }
    fn validate(preset: &Preset, interval: Option<u64>) -> Result<(), String> {
        if preset.prompt.trim().is_empty()
            || preset.prompt.len() > 32 * 1024
            || interval.is_some_and(|s| !(3600..=366 * 86400).contains(&s))
        {
            return Err(conduit::i18n::text("automation.limit"));
        }
        Ok(())
    }
    pub fn add(
        &mut self,
        mut preset: Preset,
        interval: Option<u64>,
        now: u64,
    ) -> Result<u64, String> {
        if self.schedules.len() >= 50 {
            return Err(conduit::i18n::text("automation.limit"));
        }
        Self::validate(&preset, interval)?;
        preset.permission = crate::domain::PermissionLevel::ReadOnly;
        let id = self
            .allocate()
            .ok_or_else(|| conduit::i18n::text("automation.limit"))?;
        self.schedules.push(Schedule {
            id,
            preset,
            trigger: interval.map_or(Trigger::SessionCompleted, |seconds| Trigger::Interval {
                seconds,
                next_at: now.saturating_add(seconds),
            }),
            enabled: true,
            last_event: None,
            last_claim: 0,
        });
        Ok(id)
    }
    fn editable_index(&self, id: u64, workspace: &Path) -> Result<usize, String> {
        let index = self
            .schedules
            .iter()
            .position(|s| s.id == id && s.preset.workspace == workspace)
            .ok_or_else(|| conduit::i18n::text("workflow.missing"))?;
        if self.schedules[index].enabled || self.busy(id) {
            return Err(conduit::i18n::text("automation.pause_before_edit"));
        }
        Ok(index)
    }
    pub fn edit(
        &mut self,
        id: u64,
        workspace: &Path,
        mut preset: Preset,
        interval: Option<u64>,
        now: u64,
    ) -> Result<(), String> {
        let index = self.editable_index(id, workspace)?;
        if preset.workspace != workspace {
            return Err(conduit::i18n::text("workflow.missing"));
        }
        Self::validate(&preset, interval)?;
        preset.permission = crate::domain::PermissionLevel::ReadOnly;
        let schedule = &mut self.schedules[index];
        schedule.preset = preset;
        schedule.trigger =
            interval.map_or(Trigger::SessionCompleted, |seconds| Trigger::Interval {
                seconds,
                next_at: now.saturating_add(seconds),
            });
        // Preserve the consumed event and rate limit; editing must not replay old work.
        Ok(())
    }
    pub fn remove(&mut self, id: u64, workspace: &Path) -> Result<(), String> {
        let index = self.editable_index(id, workspace)?;
        // Older run records did not retain their workspace. Backfill before removing the rule
        // so its results remain visible and isolated to the original project after restart.
        for run in self.runs.iter_mut().filter(|r| r.schedule == id) {
            run.workspace = Some(workspace.to_path_buf());
        }
        self.schedules.remove(index);
        Ok(())
    }
    pub fn busy(&self, schedule: u64) -> bool {
        self.runs
            .iter()
            .any(|r| r.schedule == schedule && r.status.active())
    }
    fn claim(&mut self, schedule: u64, now: u64, missed: bool) -> Option<u64> {
        if self.busy(schedule) {
            return None;
        }
        let id = self.allocate()?;
        if self.runs.len() >= 1000 {
            let oldest = self.runs.iter().position(|r| !r.status.active())?;
            self.runs.remove(oldest);
        }
        self.runs.push(Run {
            id,
            schedule,
            workspace: self
                .schedules
                .iter()
                .find(|s| s.id == schedule)
                .map(|s| s.preset.workspace.clone()),
            at: now,
            session: None,
            status: if missed {
                Status::Missed
            } else {
                Status::Claimed
            },
            detail: String::new(),
        });
        if let Some(s) = self.schedules.iter_mut().find(|s| s.id == schedule) {
            s.last_claim = now;
        }
        Some(id)
    }
    pub fn due(&mut self, now: u64) -> Vec<u64> {
        let due: Vec<_> = self
            .schedules
            .iter_mut()
            .filter(|s| s.enabled)
            .filter_map(|s| {
                if let Trigger::Interval { seconds, next_at } = &mut s.trigger
                    && now >= *next_at
                {
                    let missed = now.saturating_sub(*next_at) > 60;
                    *next_at = now.saturating_add(*seconds);
                    Some((s.id, missed))
                } else {
                    None
                }
            })
            .collect();
        due.into_iter()
            .filter_map(|(id, missed)| self.claim(id, now, missed))
            .collect()
    }
    pub fn event(&mut self, workspace: &Path, event: &str, now: u64) -> Vec<u64> {
        let schedules: Vec<_> = self
            .schedules
            .iter_mut()
            .filter(|s| {
                s.enabled
                    && s.preset.workspace == workspace
                    && matches!(s.trigger, Trigger::SessionCompleted)
                    && s.last_event.as_deref() != Some(event)
            })
            .filter_map(|s| {
                s.last_event = Some(event.to_owned());
                (now.saturating_sub(s.last_claim) >= 60).then_some(s.id)
            })
            .collect();
        schedules
            .into_iter()
            .filter_map(|id| self.claim(id, now, false))
            .collect()
    }
    pub fn recover(&mut self) -> bool {
        let mut changed = false;
        for run in &mut self.runs {
            if run.status.active() {
                run.status = Status::Interrupted;
                changed = true;
            }
        }
        changed
    }
    pub fn finish(&mut self, session: u64, failed: bool, cancelled: bool) -> bool {
        let Some(run) = self
            .runs
            .iter_mut()
            .find(|r| r.session == Some(session) && r.status.active())
        else {
            return false;
        };
        run.status = if cancelled {
            Status::Cancelled
        } else if failed {
            Status::Failed
        } else {
            Status::Complete
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::{PermissionLevel, Provider},
        providers::{ModelChannel, ReasoningEffort},
    };
    fn preset() -> Preset {
        Preset {
            id: 1,
            workspace: "C:/repo".into(),
            name: "Review".into(),
            prompt: "review".into(),
            provider: Provider::Codex,
            channel: ModelChannel::Cli,
            model: None,
            effort: ReasoningEffort::Default,
            acp_variant: None,
            permission: PermissionLevel::ReadOnly,
            archived: false,
        }
    }
    #[test]
    fn missed_intervals_are_recorded_without_catchup_and_events_are_deduplicated() {
        let mut scheduler = Scheduler::default();
        scheduler.add(preset(), Some(3600), 100).unwrap();
        assert!(scheduler.due(3699).is_empty());
        assert_eq!(scheduler.due(4000).len(), 1);
        assert_eq!(scheduler.runs[0].status, Status::Missed);
        assert!(scheduler.due(4000).is_empty());
        scheduler.add(preset(), None, 0).unwrap();
        assert_eq!(
            scheduler.event(Path::new("C:/repo"), "turn-1", 5000).len(),
            1
        );
        assert!(
            scheduler
                .event(Path::new("C:/repo"), "turn-1", 5001)
                .is_empty()
        );
        assert!(
            scheduler
                .event(Path::new("C:/other"), "turn-2", 5200)
                .is_empty()
        );
        assert!(
            scheduler
                .event(Path::new("C:/repo"), "turn-2", 5200)
                .is_empty()
        );
        let mut restored: Scheduler =
            serde_json::from_str(&serde_json::to_string(&scheduler).unwrap()).unwrap();
        assert!(restored.recover());
        assert!(!restored.recover());
        assert_eq!(restored.runs[1].status, Status::Interrupted);
        assert!(restored.due(5200).is_empty());
    }

    #[test]
    fn editing_and_removal_require_an_idle_paused_schedule_and_keep_results() {
        let workspace = Path::new("C:/repo");
        let mut scheduler = Scheduler::default();
        let id = scheduler.add(preset(), None, 0).unwrap();
        assert!(
            scheduler
                .edit(id, workspace, preset(), Some(7200), 100)
                .is_err()
        );
        assert!(scheduler.remove(id, workspace).is_err());
        let run = scheduler.event(workspace, "turn-1", 100)[0];
        scheduler.schedules[0].enabled = false;
        assert!(
            scheduler
                .edit(id, workspace, preset(), Some(7200), 100)
                .is_err()
        );
        assert!(scheduler.remove(id, workspace).is_err());
        scheduler.runs[0].status = Status::Complete;
        scheduler.runs[0].session = Some(42);
        let mut replacement = preset();
        replacement.prompt = "changed review".into();
        assert!(
            scheduler
                .edit(
                    id,
                    Path::new("C:/other"),
                    replacement.clone(),
                    Some(7200),
                    200
                )
                .is_err()
        );
        assert!(
            scheduler
                .edit(id, workspace, replacement.clone(), Some(1), 200)
                .is_err()
        );
        assert_eq!(scheduler.schedules[0].preset.prompt, "review");
        scheduler
            .edit(id, workspace, replacement, Some(7200), 200)
            .unwrap();
        assert!(!scheduler.schedules[0].enabled);
        assert_eq!(scheduler.schedules[0].last_event.as_deref(), Some("turn-1"));
        assert_eq!(scheduler.schedules[0].last_claim, 100);
        assert!(scheduler.due(10000).is_empty());
        assert!(matches!(
            scheduler.schedules[0].trigger,
            Trigger::Interval {
                seconds: 7200,
                next_at: 7400
            }
        ));
        // Legacy records also remain usable after the rule is deleted and data is reloaded.
        let mut legacy = serde_json::to_value(&scheduler).unwrap();
        legacy["runs"][0]
            .as_object_mut()
            .unwrap()
            .remove("workspace");
        let mut scheduler: Scheduler = serde_json::from_value(legacy).unwrap();
        assert!(scheduler.remove(id, Path::new("C:/other")).is_err());
        scheduler.remove(id, workspace).unwrap();
        let restored: Scheduler =
            serde_json::from_str(&serde_json::to_string(&scheduler).unwrap()).unwrap();
        assert!(restored.schedules.is_empty());
        assert_eq!(restored.runs[0].id, run);
        assert_eq!(restored.runs[0].session, Some(42));
        assert_eq!(restored.runs[0].workspace.as_deref(), Some(workspace));
        assert_eq!(restored.runs[0].status, Status::Complete);
        let new_id = scheduler.add(preset(), None, 0).unwrap();
        assert!(new_id > run);
    }
}
