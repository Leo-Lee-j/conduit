//! Git view and operation state have one owner and explicit dependencies.
use super::state::{CommitDraft, CommitGeneration, GitFeedback, GitOperation};
use crate::git::{self, BranchProbe, GitProbe};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    time::Instant,
};

pub(super) struct GitUi {
    pub(super) probe: Option<GitProbe>,
    pub(super) snapshot: Option<git::Snapshot>,
    pub(super) error: Option<String>,
    pub(super) workspace: Option<PathBuf>,
    pub(super) branch_probe: Option<BranchProbe>,
    pub(super) repository: Option<git::Repository>,
    pub(super) last_branch_check: Instant,
    pub(super) drafts: HashMap<PathBuf, CommitDraft>,
    pub(super) generation: Option<CommitGeneration>,
    pub(super) operation: Option<GitOperation>,
    pub(super) feedback: HashMap<PathBuf, GitFeedback>,
    pub(super) remote_tags_probe: Option<git::RemoteTagsProbe>,
    pub(super) remote_tags: HashMap<PathBuf, git::RemoteTags>,
    pub(super) pending_tag_retry: HashMap<PathBuf, git::TagRetry>,
    pub(super) collapsed_groups: HashSet<GitChangeGroup>,
    pub(super) group_by_status: bool,
    pub(super) selected_files: HashMap<PathBuf, HashSet<String>>,
    pub(super) commit_and_push: bool,
}
impl Default for GitUi {
    fn default() -> Self {
        Self {
            probe: None,
            snapshot: None,
            error: None,
            workspace: None,
            branch_probe: None,
            repository: None,
            last_branch_check: Instant::now(),
            drafts: HashMap::new(),
            generation: None,
            operation: None,
            feedback: HashMap::new(),
            remote_tags_probe: None,
            remote_tags: HashMap::new(),
            pending_tag_retry: HashMap::new(),
            collapsed_groups: HashSet::from([GitChangeGroup::Untracked]),
            group_by_status: false,
            selected_files: HashMap::new(),
            commit_and_push: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum GitChangeGroup {
    Changes,
    Untracked,
    Added,
    Modified,
    Deleted,
}

impl GitChangeGroup {
    pub(super) const ALL: [Self; 2] = [Self::Changes, Self::Untracked];
    pub(super) const STATUS: [Self; 3] = [Self::Added, Self::Modified, Self::Deleted];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Changes => "Changes",
            Self::Untracked => "Unversioned Files",
            Self::Added => "新增",
            Self::Modified => "修改",
            Self::Deleted => "删除",
        }
    }

    pub(super) fn contains(self, file: &git::ChangedFile) -> bool {
        match self {
            Self::Changes => !file.untracked(),
            Self::Untracked => file.untracked(),
            Self::Added => file.kind() == git::ChangeKind::Added,
            Self::Modified => file.kind() == git::ChangeKind::Modified,
            Self::Deleted => file.kind() == git::ChangeKind::Deleted,
        }
    }
}
