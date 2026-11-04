//! Host-owned identity and context checks shared by the interactive probe and tests.
//! This is deliberately a single trusted fixture, not the production plugin registry.
use anyhow::{Result, bail, ensure};
use serde::Serialize;

pub const OWNER: &str = "example.workspace-overview";
pub const INDEX: &str = "example.workspace-overview.index";
pub const SUMMARY: &str = "example.workspace-overview.summary";
pub const SHOW_SUMMARY: &str = "example.workspace-overview.show-summary";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Stamp {
    pub generation: u64,
    pub context_version: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    pub view_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub sessions: u32,
    pub stamp: Stamp,
    pub fail_render: bool,
}

#[derive(Debug)]
pub struct Model {
    pub active: bool,
    pub writable: bool,
    pub right_open: bool,
    pub status: String,
    pub fail_render: bool,
    pub lifecycle_callbacks: u32,
    workspace: usize,
    stamp: Stamp,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            active: false,
            writable: true,
            right_open: false,
            status: "Not activated".into(),
            fail_render: false,
            lifecycle_callbacks: 0,
            workspace: 0,
            stamp: Stamp {
                generation: 0,
                context_version: 0,
            },
        }
    }
}

impl Model {
    pub fn stamp(&self) -> Stamp {
        self.stamp
    }

    pub fn activate(&mut self) -> Result<()> {
        ensure!(self.writable, "host is read-only");
        ensure!(!self.active, "fixture is already active");
        self.stamp.generation += 1;
        self.active = true;
        self.fail_render = false;
        self.status = "Workspace overview".into();
        Ok(())
    }

    pub fn stop(&mut self) {
        self.active = false;
        self.right_open = false;
        self.fail_render = false;
        self.stamp.generation += 1;
        self.status = "Not activated".into();
    }

    pub fn select_workspace(&mut self, workspace: usize) -> Result<()> {
        ensure!(workspace < 2, "unknown fixture workspace");
        if self.workspace != workspace {
            self.workspace = workspace;
            self.stamp.context_version += 1;
            self.status = if self.active {
                "Workspace overview"
            } else {
                "Not activated"
            }
            .into();
        }
        Ok(())
    }

    pub fn set_writable(&mut self, writable: bool) {
        if self.writable != writable {
            self.writable = writable;
            self.stamp.context_version += 1;
            if !writable {
                self.stop();
            }
        }
    }

    fn check(&self, owner: &str, stamp: Stamp) -> Result<()> {
        ensure!(owner == OWNER, "owner mismatch");
        ensure!(
            self.active && self.writable,
            "fixture is inactive or read-only"
        );
        ensure!(stamp == self.stamp, "stale generation or context");
        Ok(())
    }

    pub fn snapshot(&self, owner: &str, view: &str) -> Result<Snapshot> {
        self.check(owner, self.stamp)?;
        ensure!(view == INDEX || view == SUMMARY, "unknown view");
        let (id, name, sessions) = if self.workspace == 0 {
            ("workspace-a", "Workspace A", 3)
        } else {
            ("workspace-b", "Workspace B", 7)
        };
        Ok(Snapshot {
            view_id: view.into(),
            workspace_id: id.into(),
            workspace_name: name.into(),
            sessions,
            stamp: self.stamp,
            fail_render: self.fail_render,
        })
    }

    pub fn dispatch(&mut self, owner: &str, command: &str, stamp: Stamp) -> Result<()> {
        self.check(owner, stamp)?;
        ensure!(command == SHOW_SUMMARY, "unknown fixture command");
        self.right_open = true;
        Ok(())
    }

    pub fn publish_status(&mut self, owner: &str, stamp: Stamp, text: &str) -> Result<()> {
        self.check(owner, stamp)?;
        if text.len() > 256 || text.chars().any(char::is_control) {
            bail!("status exceeds the fixture's display budget");
        }
        self.status = text.into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_state_does_not_activate_code() {
        let model = Model::default();
        assert!(model.snapshot(OWNER, INDEX).is_err());
        assert!(!model.active && !model.right_open);
    }

    #[test]
    fn footer_and_navigation_share_the_same_owned_command() {
        let mut model = Model::default();
        model.activate().unwrap();
        let stamp = model.stamp();
        assert!(model.dispatch("other.plugin", SHOW_SUMMARY, stamp).is_err());
        assert!(model.dispatch(OWNER, "other.command", stamp).is_err());
        assert!(!model.right_open);
        model.dispatch(OWNER, SHOW_SUMMARY, stamp).unwrap();
        assert!(model.right_open);
        assert_eq!(
            model.snapshot(OWNER, INDEX).unwrap().workspace_id,
            model.snapshot(OWNER, SUMMARY).unwrap().workspace_id
        );
    }

    #[test]
    fn late_result_cannot_cross_workspace_or_restart() {
        let mut model = Model::default();
        model.activate().unwrap();
        let first = model.stamp();
        model.select_workspace(1).unwrap();
        assert!(model.publish_status(OWNER, first, "late A").is_err());
        let second = model.stamp();
        model.stop();
        model.activate().unwrap();
        assert!(
            model
                .publish_status(OWNER, second, "late generation")
                .is_err()
        );
        assert_eq!(
            model.snapshot(OWNER, SUMMARY).unwrap().workspace_name,
            "Workspace B"
        );
        model
            .publish_status(OWNER, model.stamp(), "current B")
            .unwrap();
        assert_eq!(model.status, "current B");
    }

    #[test]
    fn readonly_revokes_activation_and_existing_commands() {
        let mut model = Model::default();
        model.activate().unwrap();
        let stamp = model.stamp();
        model.set_writable(false);
        assert!(model.activate().is_err());
        assert!(model.dispatch(OWNER, SHOW_SUMMARY, stamp).is_err());
        assert!(!model.active);
        model.set_writable(true);
        assert!(!model.active);
    }

    #[test]
    fn unknown_views_and_oversized_status_are_rejected() {
        let mut model = Model::default();
        model.activate().unwrap();
        assert!(model.snapshot(OWNER, "another.plugin.view").is_err());
        assert!(
            model
                .publish_status(OWNER, model.stamp(), &"x".repeat(257))
                .is_err()
        );
        assert!(
            model
                .publish_status(OWNER, model.stamp(), "line\nbreak")
                .is_err()
        );
        assert_eq!(model.status, "Workspace overview");
    }
}
