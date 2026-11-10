//! Git view and operation state have one owner and explicit dependencies.
use super::state::{CommitDraft, CommitGeneration, GitOperation};
use crate::git::{self, BranchProbe, GitProbe};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};

pub(super) struct GitUi {
    pub(super) conflict_browsers:
        HashMap<PathBuf, gpui_kit::Entity<super::git_conflicts::ConflictBrowser>>,
    pub(super) announced_conflicts: HashMap<PathBuf, Vec<String>>,
    pub(super) probe: Option<GitProbe>,
    pub(super) auto_stage_after_refresh: bool,
    pub(super) snapshot: Option<git::Snapshot>,
    pub(super) error: Option<String>,
    pub(super) workspace: Option<PathBuf>,
    pub(super) branch_probe: Option<BranchProbe>,
    pub(super) repository: Option<git::Repository>,
    pub(super) last_branch_check: Instant,
    pub(super) drafts: HashMap<PathBuf, CommitDraft>,
    pub(super) generation: Option<CommitGeneration>,
    pub(super) operation: Option<GitOperation>,
    pub(super) task_worktree: Option<crate::workflow::WorkItem>,
    pub(super) task_cleanup: Option<PathBuf>,
    pub(super) local_tags: HashMap<PathBuf, Vec<String>>,
    pub(super) remote_tags_checked: HashSet<PathBuf>,
    pub(super) remote_tags_probe: Option<git::RemoteTagsProbe>,
    pub(super) remote_tags: HashMap<PathBuf, git::RemoteTags>,
    pub(super) pending_tag_retry: HashMap<PathBuf, git::TagRetry>,
    pub(super) collapsed_groups: HashSet<GitChangeGroup>,
    pub(super) group_by_status: bool,
    pub(super) selected_files: HashMap<PathBuf, HashSet<String>>,
}
impl Default for GitUi {
    fn default() -> Self {
        Self {
            conflict_browsers: HashMap::new(),
            announced_conflicts: HashMap::new(),
            probe: None,
            auto_stage_after_refresh: false,
            snapshot: None,
            error: None,
            workspace: None,
            branch_probe: None,
            repository: None,
            last_branch_check: Instant::now(),
            drafts: HashMap::new(),
            generation: None,
            operation: None,
            task_worktree: None,
            task_cleanup: None,
            local_tags: HashMap::new(),
            remote_tags_checked: HashSet::new(),
            remote_tags_probe: None,
            remote_tags: HashMap::new(),
            pending_tag_retry: HashMap::new(),
            collapsed_groups: HashSet::from([GitChangeGroup::Untracked]),
            group_by_status: false,
            selected_files: HashMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum GitChangeGroup {
    Conflicts,
    Changes,
    Untracked,
    Added,
    Modified,
    Deleted,
}

impl GitChangeGroup {
    pub(super) const ALL: [Self; 3] = [Self::Conflicts, Self::Changes, Self::Untracked];
    pub(super) const STATUS: [Self; 4] =
        [Self::Conflicts, Self::Added, Self::Modified, Self::Deleted];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Conflicts => conduit::i18n::text_static("git_conflicts.title"),
            Self::Changes => "Changes",
            Self::Untracked => "Unversioned Files",
            Self::Added => conduit::i18n::text_static("git_state.added"),
            Self::Modified => conduit::i18n::text_static("git_state.modified"),
            Self::Deleted => conduit::i18n::text_static("git_state.deleted"),
        }
    }

    pub(super) fn contains(self, file: &git::ChangedFile) -> bool {
        if file.conflicted() {
            return self == Self::Conflicts;
        }
        match self {
            Self::Conflicts => false,
            Self::Changes => !file.untracked(),
            Self::Untracked => file.untracked(),
            Self::Added => file.kind() == git::ChangeKind::Added,
            Self::Modified => file.kind() == git::ChangeKind::Modified,
            Self::Deleted => file.kind() == git::ChangeKind::Deleted,
        }
    }
}
