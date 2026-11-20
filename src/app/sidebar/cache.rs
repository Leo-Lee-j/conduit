//! Lightweight navigation metadata; transcripts stay in the store.
use super::*;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::menu::PopupMenu;
use std::rc::Rc;

const INACTIVE_WORKSPACE_AGE: u64 = 3 * 24 * 60 * 60;
pub(super) const WORKSPACE_MOVE_DURATION: Duration = Duration::from_millis(240);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionGroup {
    Pinned,
    Day,
    Week,
    Older,
}

impl SessionGroup {
    fn new(pinned: bool, activity: u64, now: u64) -> Self {
        if pinned {
            Self::Pinned
        } else if activity > 0 && now.saturating_sub(activity) < 24 * 60 * 60 {
            Self::Day
        } else if activity > 0 && now.saturating_sub(activity) < 7 * 24 * 60 * 60 {
            Self::Week
        } else {
            Self::Older
        }
    }

    pub(super) fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Pinned => "sidebar.group_pinned",
            Self::Day => "sidebar.group_day",
            Self::Week => "sidebar.group_week",
            Self::Older => "sidebar.group_older",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Row {
    Project {
        key: PathBuf,
        name: String,
        copies: usize,
        count: usize,
        activity: Option<(u64, bool)>,
    },
    Workspace {
        path: PathBuf,
        count: usize,
        last: Option<u64>,
    },
    Loading(PathBuf),
    Group {
        path: PathBuf,
        group: SessionGroup,
    },
    Session {
        path: PathBuf,
        id: u64,
    },
    More {
        path: PathBuf,
        total: usize,
        expanded: bool,
    },
    Empty,
}
impl Row {
    pub(super) fn path(&self) -> Option<&Path> {
        match self {
            Self::Project { key: path, .. }
            | Self::Workspace { path, .. }
            | Self::Group { path, .. }
            | Self::Session { path, .. }
            | Self::More { path, .. }
            | Self::Loading(path) => Some(path),
            _ => None,
        }
    }
    pub(super) fn height(&self) -> f32 {
        match self {
            Self::Group { .. } => 22.,
            // Two 16px title lines with 2px of padding above and below.
            Self::Session { .. } => 36.,
            Self::More { .. } => 24.,
            Self::Empty => 76.,
            _ => ROW_HEIGHT,
        }
    }
    pub(super) fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Group { path: a, group: ga }, Self::Group { path: b, group: gb }) => {
                a == b && ga == gb
            }
            (Self::Session { id: a, .. }, Self::Session { id: b, .. }) => a == b,
            _ => {
                std::mem::discriminant(self) == std::mem::discriminant(other)
                    && self.path() == other.path()
            }
        }
    }
    /// Own the inter-row space so tree guides stay inside virtualized bounds.
    pub(super) fn list_height(&self) -> f32 {
        self.height() + 2.
    }

    pub(super) fn is_header(&self) -> bool {
        matches!(self, Self::Project { .. } | Self::Workspace { .. })
    }
}

#[derive(PartialEq, Eq)]
struct Summary {
    title: String,
    provider: Provider,
    native_id: Option<String>,
    is_subagent: bool,
    archived: bool,
    pinned: bool,
    activity: u64,
    running: bool,
    has_user: bool,
}
impl Summary {
    fn new(session: &SessionRecord, control: &session_controller::SessionController) -> Self {
        Self {
            title: session.title.clone(),
            provider: session.provider,
            native_id: session.native_id.clone(),
            is_subagent: session.is_subagent,
            archived: session.archived,
            pinned: session.pinned,
            activity: session.last_activity_at(),
            running: control.runs.contains_key(&session.id),
            has_user: session.has_user_messages(),
        }
    }
    fn sort_key(&self, id: u64) -> std::cmp::Reverse<(bool, u64, u64)> {
        std::cmp::Reverse((self.pinned, self.activity, id))
    }
}
#[derive(Default)]
struct WorkspaceRows {
    summaries: HashMap<u64, Summary>,
    ordered: Vec<u64>,
    needs_sort: bool,
}
impl WorkspaceRows {
    fn new(workspace: &Workspace, control: &session_controller::SessionController) -> Self {
        Self {
            summaries: workspace
                .sessions
                .iter()
                .map(|s| (s.id, Summary::new(s, control)))
                .collect(),
            ordered: workspace.sessions.iter().map(|s| s.id).collect(),
            needs_sort: true,
        }
    }
}

/// Small state only: comparing this never walks session bodies or all sessions.
#[derive(PartialEq, Eq)]
pub(super) struct Inputs {
    workspaces: Vec<(PathBuf, usize)>,
    expanded: HashSet<usize>,
    collapsed: HashSet<PathBuf>,
    expanded_projects: HashSet<PathBuf>,
    auto_expanded: HashSet<PathBuf>,
    auto_collapse_inactive: bool,
    auto_sort_workspaces: bool,
    loading: HashSet<PathBuf>,
    runs: HashSet<u64>,
    task_states: HashMap<u64, TaskState>,
    unread: HashSet<u64>,
    namings: HashSet<u64>,
    progress: HashMap<PathBuf, (usize, usize)>,
    query: String,
    results: u64,
    show_archived: bool,
    limit: usize,
    selected_workspace: Option<usize>,
    selected_session: Option<u64>,
    delete_target: Option<u64>,
    keyboard_row: Option<Row>,
    remove_target: Option<usize>,
    editable: bool,
    sidebar_collapsed: bool,
    provider: Provider,
    appearance: (u32, u32, u32),
    age_interval: u64,
    age_tick: u64,
}
impl Inputs {
    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn at_time(mut self, time: u64) -> Self {
        self.age_interval = 1;
        self.age_tick = time;
        self
    }

    pub(super) fn new(app: &ConduitApp) -> Self {
        let now = unix_now();
        let age_interval = if app.sidebar.has_recent_activity(now) {
            1
        } else {
            60
        };
        Self {
            keyboard_row: app.sidebar.keyboard_row.clone(),
            workspaces: app
                .store
                .workspaces
                .iter()
                .map(|w| (w.path.clone(), w.sessions.len()))
                .collect(),
            expanded: app.expanded_workspaces.clone(),
            collapsed: app.collapsed_workspaces.clone(),
            expanded_projects: app.sidebar.expanded_projects.clone(),
            auto_expanded: app.sidebar.auto_expanded.clone(),
            auto_collapse_inactive: app.store.ui.auto_collapse_inactive_workspaces,
            auto_sort_workspaces: app.store.ui.auto_sort_workspaces,
            loading: app.history_loading.clone(),
            runs: app.session_control.runs.keys().copied().collect(),
            task_states: app
                .session_control
                .runs
                .iter()
                .map(|(id, run)| (*id, TaskState::new(run)))
                .collect(),
            unread: app.session_control.unread_completed.clone(),
            namings: app.session_control.namings.keys().copied().collect(),
            progress: app.session_control.scan_progress.clone(),
            query: app.search_query.trim().to_lowercase(),
            results: app.search_ui.result_generation,
            show_archived: app.show_archived,
            limit: app.store.ui.sessions_per_workspace.clamp(1, 30),
            selected_workspace: app.selected_workspace,
            selected_session: app.selected_session,
            delete_target: app.delete_target,
            remove_target: app.remove_workspace_target,
            editable: app.writable && !app.closing,
            sidebar_collapsed: app.plugins.collapsed(),
            provider: app.new_session_provider,
            appearance: (palette().canvas, palette().sidebar_bg, palette().text),
            age_interval,
            age_tick: now / age_interval,
        }
    }
    fn same_rows(&self, other: &Self) -> bool {
        self.workspaces == other.workspaces
            && self.expanded == other.expanded
            && self.collapsed == other.collapsed
            && self.expanded_projects == other.expanded_projects
            && self.unread == other.unread
            && self.auto_expanded == other.auto_expanded
            && self.auto_collapse_inactive == other.auto_collapse_inactive
            && self.auto_sort_workspaces == other.auto_sort_workspaces
            && self.loading == other.loading
            && self.query == other.query
            && self.results == other.results
            && self.show_archived == other.show_archived
            && self.limit == other.limit
            // Group labels must cross their age boundaries even without new messages.
            && self.age_tick * self.age_interval / 60 == other.age_tick * other.age_interval / 60
    }
}

pub(in crate::app) struct SidebarState {
    pub(super) focus: Option<FocusHandle>,
    pub(super) keyboard_row: Option<Row>,
    pub(super) menu: Option<Entity<PopupMenu>>,
    pub(super) menu_subscription: Option<Subscription>,
    pub(super) rows: Rc<Vec<Row>>,
    pub(super) sizes: Rc<Vec<gpui_kit::Size<Pixels>>>,
    pub(in crate::app) scroll: VirtualListScrollHandle,
    // session_mut records candidates; only changed navigation fields invalidate rows.
    pub(in crate::app) dirty_sessions: HashSet<u64>,
    dirty_workspaces: HashSet<PathBuf>,
    groups: HashMap<PathBuf, WorkspaceRows>,
    pub(super) auto_expanded: HashSet<PathBuf>,
    pub(super) auto_collapsed: HashSet<PathBuf>,
    pub(super) projects: projects::Groups,
    pub(super) expanded_projects: HashSet<PathBuf>,
    indices: HashMap<PathBuf, usize>,
    inputs: Option<Inputs>,
    latest_activity: u64,
    origins: Vec<f32>,
    workspace_headers: Vec<usize>,
    workspace_motion: HashMap<PathBuf, f32>,
    pub(super) motion_started: Option<Instant>,
    pub(super) motion_generation: u64,
    #[cfg(test)]
    pub(super) rendered_rows: std::cell::Cell<usize>,
    #[cfg(test)]
    pub(super) sort_count: usize,
    #[cfg(test)]
    pub(super) rebuild_count: usize,
}
impl Default for SidebarState {
    fn default() -> Self {
        Self {
            focus: None,
            keyboard_row: None,
            menu: None,
            menu_subscription: None,
            rows: Rc::default(),
            sizes: Rc::default(),
            scroll: VirtualListScrollHandle::new(),
            dirty_sessions: HashSet::new(),
            dirty_workspaces: HashSet::new(),
            groups: HashMap::new(),
            auto_expanded: HashSet::new(),
            auto_collapsed: HashSet::new(),
            projects: projects::Groups::default(),
            expanded_projects: HashSet::new(),
            indices: HashMap::new(),
            inputs: None,
            latest_activity: 0,
            origins: Vec::new(),
            workspace_headers: Vec::new(),
            workspace_motion: HashMap::new(),
            motion_started: None,
            motion_generation: 0,
            #[cfg(test)]
            rendered_rows: std::cell::Cell::new(0),
            #[cfg(test)]
            sort_count: 0,
            #[cfg(test)]
            rebuild_count: 0,
        }
    }
}
impl SidebarState {
    /// Resolve against the live scroll offset, including scrollbar and keyboard scrolling.
    /// Only header indices are searched, independent of the number of sessions.
    pub(super) fn sticky_workspace(&self) -> Option<(Row, f32)> {
        let y = -f32::from(self.scroll.offset().y);
        let position = self
            .workspace_headers
            .partition_point(|&index| self.origins[index] < y)
            .checked_sub(1)?;
        let index = self.workspace_headers[position];
        let row = &self.rows[index];
        // Empty/collapsed groups have no children to keep in context.
        if self.rows.get(index + 1).is_none_or(|next| match row {
            Row::Project { key, .. } => {
                next.path().and_then(|path| self.projects.parent(path)) != Some(key)
                    || matches!(next, Row::Project { .. })
            }
            _ => next.is_header() || next.path() != row.path(),
        }) {
            return None;
        }
        let offset = self
            .workspace_headers
            .get(position + 1)
            .map_or(0., |&next| (self.origins[next] - y - ROW_HEIGHT).min(0.));
        Some((row.clone(), offset))
    }

    pub(super) fn reveal_workspace(&mut self, path: &Path) {
        if let Some(&index) = self.workspace_headers.iter().find(
            |&&index| matches!(&self.rows[index], Row::Workspace { path: p, .. } if p == path),
        ) {
            self.keyboard_row = Some(self.rows[index].clone());
            // Direct offset also works before the virtual list has laid out its new rows.
            self.scroll
                .set_offset(point(px(0.), px(-self.origins[index])));
        }
    }

    pub(super) fn reveal_project(&mut self, key: &Path) {
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| matches!(row, Row::Project { key: k, .. } if k == key))
        {
            self.keyboard_row = Some(self.rows[index].clone());
            self.scroll
                .set_offset(point(px(0.), px(-self.origins[index])));
        }
    }

    pub(super) fn reveal_keyboard_row(&self, index: usize) {
        // Child focus must remain below its sticky parent, including the More row.
        let inset = if matches!(self.rows[index], Row::Session { .. } | Row::More { .. }) {
            ROW_HEIGHT
        } else {
            0.
        };
        self.scroll
            .set_offset(point(px(0.), px(-(self.origins[index] - inset).max(0.))));
    }

    pub(super) fn motion_offset(&self, path: &Path) -> Option<f32> {
        let elapsed =
            self.motion_started?.elapsed().as_secs_f32() / WORKSPACE_MOVE_DURATION.as_secs_f32();
        (elapsed < 1.)
            .then(|| self.motion_start_offset(path).unwrap_or(0.) * (1. - elapsed).powi(3))
            .filter(|offset| offset.abs() > 0.1)
    }

    pub(super) fn motion_start_offset(&self, path: &Path) -> Option<f32> {
        self.workspace_motion
            .get(path)
            .copied()
            .filter(|offset| offset.abs() > 0.1)
    }

    fn has_recent_activity(&self, now: u64) -> bool {
        self.latest_activity != 0 && now.saturating_sub(self.latest_activity) < 60
    }
    pub(in crate::app) fn age_refresh_interval(&self) -> u64 {
        if self.has_recent_activity(unix_now())
            || self
                .inputs
                .as_ref()
                .is_some_and(|inputs| inputs.age_interval == 1)
        {
            1
        } else {
            60
        }
    }
    pub(in crate::app) fn invalidate_workspace(&mut self, path: &Path) {
        self.dirty_workspaces.insert(path.to_owned());
    }
    pub(super) fn workspace_index(&self, path: &Path) -> Option<usize> {
        self.indices.get(path).copied()
    }
    pub(super) fn refresh(
        &mut self,
        store: &Store,
        control: &session_controller::SessionController,
        search: &search_worker::SearchUi,
        mut inputs: Inputs,
    ) -> bool {
        let selected_path = inputs
            .selected_workspace
            .and_then(|index| store.workspaces.get(index))
            .map(|workspace| &workspace.path);
        let previous_selected_project = selected_path
            .and_then(|path| self.projects.parent(path))
            .cloned();
        let mut changed = self.inputs.as_ref() != Some(&inputs);
        let mut rebuild = self
            .inputs
            .as_ref()
            .is_none_or(|old| !old.same_rows(&inputs));
        if let Some(old) = &self.inputs {
            self.dirty_sessions
                .extend(old.runs.symmetric_difference(&inputs.runs).copied());
        }
        if self
            .inputs
            .as_ref()
            .is_none_or(|old| old.workspaces != inputs.workspaces)
        {
            self.projects = projects::Groups::new(
                store
                    .workspaces
                    .iter()
                    .map(|workspace| workspace.path.clone()),
            );
            self.expanded_projects
                .retain(|key| self.projects.contains(key));
            inputs.expanded_projects = self.expanded_projects.clone();
            self.indices = store
                .workspaces
                .iter()
                .enumerate()
                .map(|(i, w)| (w.path.clone(), i))
                .collect();
            self.groups
                .retain(|path, _| self.indices.contains_key(path));
            self.auto_expanded
                .retain(|path| self.indices.contains_key(path));
        }
        // Opening a session through a task card, tab or search reveals its group.
        // A subsequent manual group collapse is respected while selection is unchanged.
        if (self.inputs.as_ref().is_none_or(|old| {
            old.selected_workspace != inputs.selected_workspace
                || old.selected_session != inputs.selected_session
        }) || selected_path.and_then(|path| self.projects.parent(path))
            != previous_selected_project.as_ref())
            && let Some(workspace) = inputs
                .selected_workspace
                .and_then(|index| store.workspaces.get(index))
            && let Some(key) = self.projects.parent(&workspace.path)
        {
            self.expanded_projects.insert(key.clone());
            inputs.expanded_projects = self.expanded_projects.clone();
            rebuild = true;
            changed = true;
        }
        for workspace in &store.workspaces {
            if self.dirty_workspaces.contains(&workspace.path)
                || self
                    .groups
                    .get(&workspace.path)
                    .is_none_or(|g| g.summaries.len() != workspace.sessions.len())
            {
                self.groups.insert(
                    workspace.path.clone(),
                    WorkspaceRows::new(workspace, control),
                );
                changed = true;
                rebuild = true;
            }
        }
        self.dirty_workspaces.clear();
        for id in std::mem::take(&mut self.dirty_sessions) {
            let Some((w, s)) = control.locate(&store.workspaces, id) else {
                continue;
            };
            let workspace = &store.workspaces[w];
            let group = self
                .groups
                .get_mut(&workspace.path)
                .expect("workspace cached");
            let next = Summary::new(&workspace.sessions[s], control);
            let previous = group.summaries.get(&id);
            if previous != Some(&next) {
                let order_changed =
                    previous.is_none_or(|old| old.sort_key(id) != next.sort_key(id));
                rebuild |= order_changed
                    || previous.is_none_or(|old| {
                        old.archived != next.archived
                            || old.is_subagent != next.is_subagent
                            || old.running != next.running
                    });
                group.needs_sort |= order_changed;
                group.summaries.insert(id, next);
                changed = true;
            }
        }
        // Use cached summaries, including unloaded transcript activity. Recheck
        // on age ticks so a long-running app also crosses the three-day boundary.
        if changed || rebuild {
            let now = inputs.age_tick * inputs.age_interval;
            let auto_collapsed = if inputs.auto_collapse_inactive && inputs.query.is_empty() {
                self.groups
                    .iter()
                    .filter(|(path, group)| {
                        self.indices.get(*path).copied() != inputs.selected_workspace
                            && !inputs.auto_expanded.contains(*path)
                            && !group
                                .summaries
                                .values()
                                .filter(|s| !s.is_subagent)
                                .any(|s| {
                                    s.running
                                        || (s.activity > 0
                                            && now.saturating_sub(s.activity)
                                                < INACTIVE_WORKSPACE_AGE)
                                })
                    })
                    .map(|(path, _)| path.clone())
                    .collect()
            } else {
                HashSet::new()
            };
            if self.auto_collapsed != auto_collapsed {
                self.auto_collapsed = auto_collapsed;
                rebuild = true;
                changed = true;
            }
        }
        if rebuild {
            let mut rows = Vec::new();
            // Sort only the display projection: all stored workspace indices and
            // the user's manual order remain valid. Stable ties keep that order.
            let mut workspaces: Vec<_> = store.workspaces.iter().enumerate().collect();
            if inputs.auto_sort_workspaces {
                workspaces.sort_by_cached_key(|(_, workspace)| {
                    let summaries = &self.groups[&workspace.path].summaries;
                    let visible = || summaries.values().filter(|s| !s.is_subagent);
                    let running = visible().any(|s| s.running);
                    let activity = visible().map(|s| s.activity).max().unwrap_or(0);
                    std::cmp::Reverse((running, activity))
                });
            }
            for (index, workspace) in workspaces {
                let group = self
                    .groups
                    .get_mut(&workspace.path)
                    .expect("workspace cached");
                if group.needs_sort {
                    group
                        .ordered
                        .sort_unstable_by_key(|id| group.summaries[id].sort_key(*id));
                    group.needs_sort = false;
                    #[cfg(test)]
                    {
                        self.sort_count += 1;
                    }
                }
                let filtered: Vec<_> = group
                    .ordered
                    .iter()
                    .copied()
                    .filter(|id| {
                        !group.summaries[id].is_subagent
                            && (inputs.show_archived || !group.summaries[id].archived)
                            && (inputs.query.is_empty()
                                || (search.result_query == inputs.query
                                    && search.matches.contains(id)))
                    })
                    .collect();
                if !inputs.query.is_empty() && filtered.is_empty() {
                    continue;
                }
                let path = &workspace.path;
                let total = filtered.len();
                rows.push(Row::Workspace {
                    path: path.clone(),
                    count: total,
                    last: filtered
                        .first()
                        .copied()
                        .or_else(|| workspace.visible_sessions().next().map(|s| s.id)),
                });
                if inputs.collapsed.contains(path) || self.auto_collapsed.contains(path) {
                    continue;
                }
                if inputs.loading.contains(path) {
                    rows.push(Row::Loading(path.clone()));
                }
                let expanded = inputs.expanded.contains(&index);
                let pinned = filtered
                    .iter()
                    .take_while(|id| group.summaries[id].pinned)
                    .count();
                let ordinary = total - pinned;
                let mut previous_group = None;
                for id in filtered.into_iter().take(if expanded {
                    total
                } else {
                    pinned + inputs.limit
                }) {
                    let summary = &group.summaries[&id];
                    let session_group = SessionGroup::new(
                        summary.pinned,
                        summary.activity,
                        inputs.age_tick * inputs.age_interval,
                    );
                    if previous_group != Some(session_group) {
                        rows.push(Row::Group {
                            path: path.clone(),
                            group: session_group,
                        });
                        previous_group = Some(session_group);
                    }
                    rows.push(Row::Session {
                        path: path.clone(),
                        id,
                    });
                }
                if ordinary > inputs.limit {
                    rows.push(Row::More {
                        path: path.clone(),
                        total: ordinary,
                        expanded,
                    });
                }
            }
            if store.workspaces.is_empty() {
                rows.push(Row::Empty);
            }
            let mut attention = HashMap::new();
            for (&id, running) in inputs
                .unread
                .iter()
                .map(|id| (id, false))
                .chain(inputs.runs.iter().map(|id| (id, true)))
            {
                if let Some((index, row)) = control.locate(&store.workspaces, id) {
                    if store.workspaces[index].sessions[row].is_subagent {
                        continue;
                    }
                    let entry = attention
                        .entry(store.workspaces[index].path.clone())
                        .or_insert((id, running));
                    if (!running, id) < (!entry.1, entry.0) {
                        *entry = (id, running);
                    }
                }
            }
            let rows = self.projects.project(
                rows,
                &inputs.expanded_projects,
                &attention,
                !inputs.query.is_empty(),
            );
            if self.rows.as_ref() != &rows {
                let reset = self
                    .inputs
                    .as_ref()
                    .is_some_and(|old| old.query != inputs.query);
                self.replace_rows(rows, reset);
            }
            #[cfg(test)]
            {
                self.rebuild_count += 1;
            }
        }
        if changed {
            self.latest_activity = self
                .rows
                .iter()
                .filter_map(|row| {
                    let Row::Session { path, id } = row else {
                        return None;
                    };
                    self.groups
                        .get(path)?
                        .summaries
                        .get(id)
                        .map(|summary| summary.activity)
                })
                .max()
                .unwrap_or(0);
        }
        self.inputs = Some(inputs);
        changed
    }
    /// Keep the top row and its pixel offset when inserts/reorders happen above it.
    fn replace_rows(&mut self, rows: Vec<Row>, reset: bool) {
        let old_workspace_positions: HashMap<_, _> = self
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                Row::Workspace { path, .. } => Some((path.clone(), self.origins[index])),
                _ => None,
            })
            .collect();
        let old_order: Vec<_> = self
            .rows
            .iter()
            .filter_map(|row| match row {
                Row::Workspace { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        let new_order: Vec<_> = rows
            .iter()
            .filter_map(|row| match row {
                Row::Workspace { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        // Animate reorders, not initial population, filtering or expanding groups.
        let reordered = !reset
            && self.projects.is_empty()
            && old_order.len() == new_order.len()
            && old_order != new_order
            && new_order
                .iter()
                .all(|path| old_workspace_positions.contains_key(*path));
        let y = -f32::from(self.scroll.offset().y);
        let top = self
            .origins
            .partition_point(|origin| *origin <= y)
            .saturating_sub(1);
        let anchor = self.rows.get(top).map(|row| (row, y - self.origins[top]));
        let mut origin = 0.;
        let origins: Vec<_> = rows
            .iter()
            .map(|row| {
                let current = origin;
                origin += row.list_height();
                current
            })
            .collect();
        if reset {
            self.scroll.set_offset(point(px(0.), px(0.)));
        } else if y > 0.
            && let Some((anchor, offset)) = anchor
        {
            let next = rows
                .iter()
                .position(|row| row.same_identity(anchor))
                .map(|index| (index, offset))
                .or_else(|| {
                    rows.iter()
                        .position(|row| {
                            matches!(row, Row::Workspace { .. }) && row.path() == anchor.path()
                        })
                        .map(|index| (index, 0.))
                })
                .or_else(|| {
                    let parent = anchor.path().and_then(|path| self.projects.parent(path))?;
                    rows.iter()
                        .position(|row| matches!(row, Row::Project { key, .. } if key == parent))
                        .map(|index| (index, 0.))
                });
            if let Some((index, offset)) = next {
                self.scroll
                    .set_offset(point(px(0.), px(-origins[index] - offset)));
            }
        }
        let next_scroll = f32::from(self.scroll.offset().y);
        let motion = if reordered {
            rows.iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    let Row::Workspace { path, .. } = row else {
                        return None;
                    };
                    let previous = old_workspace_positions.get(path)?;
                    let offset = previous - y - origins[index] - next_scroll
                        + self.motion_offset(path).unwrap_or(0.);
                    // Bound travel for virtualized/offscreen groups so they don't
                    // spend the entire transition outside the visible list.
                    Some((
                        path.clone(),
                        offset.clamp(-ROW_HEIGHT * 2., ROW_HEIGHT * 2.),
                    ))
                })
                .collect()
        } else {
            HashMap::new()
        };
        self.workspace_motion = motion;
        self.motion_started = reordered.then(Instant::now);
        if reordered {
            self.motion_generation = self.motion_generation.wrapping_add(1);
        }
        // Reuse the allocation when only identities change; layout sizes remain valid.
        let sizes: Vec<_> = rows
            .iter()
            .map(|row| size(px(0.), px(row.list_height())))
            .collect();
        if self.sizes.as_ref() != &sizes {
            self.sizes = Rc::new(sizes);
        }
        self.origins = origins;
        self.workspace_headers = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| row.is_header().then_some(index))
            .collect();
        if let Some(focused) = &self.keyboard_row
            && !rows.iter().any(|row| row.same_identity(focused))
        {
            self.keyboard_row = rows
                .iter()
                .find(|row| matches!(row, Row::Workspace { .. }) && row.path() == focused.path())
                .or_else(|| {
                    let parent = focused.path().and_then(|path| self.projects.parent(path))?;
                    rows.iter()
                        .find(|row| matches!(row, Row::Project { key, .. } if key == parent))
                })
                .cloned();
        }
        self.rows = Rc::new(rows);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{INACTIVE_WORKSPACE_AGE, Inputs, Row};
    use crate::app::{ConduitApp, Store, Workspace};
    use gpui_kit::{AppContext, TestAppContext};

    #[gpui_kit::test]
    fn subagent_visibility_refreshes_cached_rows_counts_and_empty_workspace_target(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "subagent-sidebar");
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                app.store.workspaces.truncate(1);
                app.store.workspaces[0].sessions.truncate(1);
                let mut child = app.store.workspaces[0].sessions[0].clone();
                child.id = 2;
                child.pinned = true;
                app.store.workspaces[0].sessions.push(child);
                let path = app.store.workspaces[0].path.clone();
                app.sidebar.invalidate_workspace(&path);
                app.refresh_sidebar();
                assert_eq!(
                    app.sidebar
                        .rows
                        .iter()
                        .filter(|r| matches!(r, Row::Session { .. }))
                        .count(),
                    2
                );
                app.session_mut(2).unwrap().is_subagent = true;
                app.refresh_sidebar();
                assert!(
                    !app.sidebar
                        .rows
                        .iter()
                        .any(|r| matches!(r, Row::Session { id: 2, .. }))
                );
                assert!(app.sidebar.rows.iter().any(|r| matches!(
                    r,
                    Row::Workspace {
                        count: 1,
                        last: Some(1),
                        ..
                    }
                )));
                app.show_archived = true;
                app.session_mut(2).unwrap().archived = true;
                app.refresh_sidebar();
                assert!(
                    !app.sidebar
                        .rows
                        .iter()
                        .any(|r| matches!(r, Row::Session { id: 2, .. }))
                );
                app.session_mut(1).unwrap().is_subagent = true;
                app.refresh_sidebar();
                assert!(app.sidebar.rows.iter().any(|r| matches!(
                    r,
                    Row::Workspace {
                        count: 0,
                        last: None,
                        ..
                    }
                )));
            });
        });
    }

    #[gpui_kit::test]
    fn long_workspace_can_return_to_summary_without_switching_chat(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let path = std::env::temp_dir().join("conduit-sidebar-long-workspace");
                let sessions = (1..=100)
                    .map(|id| {
                        let mut session = crate::app::tests::session();
                        session.id = id;
                        session.created_at = id;
                        session.pinned = id == 100;
                        session
                    })
                    .collect();
                let store = Store {
                    workspaces: vec![
                        Workspace {
                            path: path.clone(),
                            sessions,
                        },
                        Workspace {
                            path: path.join("other"),
                            sessions: vec![],
                        },
                    ],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.startup = None;
                app.store.ui.auto_sort_workspaces = false;
                app.store.ui.sessions_per_workspace = 3;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.refresh_sidebar();
                app.set_sidebar_sessions_expanded(&path, true, window, cx);
                let middle = app
                    .sidebar
                    .rows
                    .iter()
                    .position(|row| matches!(row, Row::Session { id: 50, .. }))
                    .unwrap();
                app.sidebar.scroll.set_offset(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-app.sidebar.origins[middle]),
                ));
                let (sticky, _) = app
                    .sidebar
                    .sticky_workspace()
                    .expect("header stays reachable");
                assert_eq!(sticky.path(), Some(path.as_path()));

                app.set_sidebar_sessions_expanded(&path, false, window, cx);
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.selected_workspace, Some(0));
                let visible: Vec<_> = app
                    .sidebar
                    .rows
                    .iter()
                    .filter_map(|row| {
                        if let Row::Session { id, .. } = row {
                            Some(*id)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(visible, vec![100, 99, 98, 97]);
                assert!(matches!(app.sidebar.keyboard_row.as_ref(),
                    Some(Row::Workspace { path: focused, .. }) if focused == &path));
                let top = app.sidebar.scroll.offset().y;
                let header = app.sidebar.workspace_headers[0];
                assert_eq!(f32::from(top), -app.sidebar.origins[header]);
                assert!(
                    app.sidebar.sticky_workspace().is_none(),
                    "natural header is visible"
                );

                app.set_sidebar_sessions_expanded(&path, true, window, cx);
                app.sidebar.scroll.set_offset(gpui_kit::point(
                    gpui_kit::px(0.),
                    gpui_kit::px(-app.sidebar.origins[middle]),
                ));
                app.toggle_sidebar_workspace(&path, window, cx);
                assert!(app.workspace_collapsed(&path));
                assert_eq!(app.selected_session, Some(1));
                assert!(
                    !app.sidebar
                        .rows
                        .iter()
                        .any(|row| matches!(row, Row::Session { .. }))
                );
                assert!(app.sidebar.sticky_workspace().is_none());
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }

    #[gpui_kit::test]
    fn inactive_workspaces_follow_activity_and_preserve_manual_choices(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let now = 1_000_000;
                let mut old = crate::app::tests::session();
                old.created_at = now - INACTIVE_WORKSPACE_AGE;
                let mut recent = old.clone();
                recent.id = 2;
                // A cold transcript's recent activity must override old creation.
                recent.transcript.loaded = false;
                recent.transcript.reference = Some(
                    serde_json::from_value(serde_json::json!({
                        "hash": "test-reference",
                        "message_count": 1,
                        "last_activity_at": now - 1,
                        "has_user": true,
                        "bytes": 0,
                        "attachments": []
                    }))
                    .unwrap(),
                );
                let root = std::env::temp_dir().join("conduit-inactive-workspaces");
                let store = Store {
                    workspaces: vec![
                        Workspace {
                            path: root.join("old"),
                            sessions: vec![old],
                        },
                        Workspace {
                            path: root.join("recent"),
                            sessions: vec![recent],
                        },
                        Workspace {
                            path: root.join("empty"),
                            sessions: vec![],
                        },
                    ],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.startup = None;
                app.selected_workspace = None;
                app.selected_session = None;
                let old_path = root.join("old");
                let recent_path = root.join("recent");
                let empty_path = root.join("empty");
                let refresh = |app: &mut ConduitApp, time| {
                    let mut inputs = Inputs::new(app);
                    inputs.age_interval = 1;
                    inputs.age_tick = time;
                    app.sidebar
                        .refresh(&app.store, &app.session_control, &app.search_ui, inputs);
                };
                refresh(&mut app, now);
                assert!(app.sidebar.auto_collapsed.is_empty());
                let order = |app: &ConduitApp| -> Vec<_> {
                    app.sidebar
                        .rows
                        .iter()
                        .filter_map(|row| match row {
                            Row::Workspace { path, .. } => Some(path.clone()),
                            _ => None,
                        })
                        .collect()
                };
                let manual_order = vec![old_path.clone(), recent_path.clone(), empty_path.clone()];
                assert_eq!(
                    order(&app),
                    vec![recent_path.clone(), old_path.clone(), empty_path.clone()]
                );
                // Automatic display order never mutates stored indices or manual order.
                assert_eq!(
                    app.store
                        .workspaces
                        .iter()
                        .map(|w| w.path.clone())
                        .collect::<Vec<_>>(),
                    manual_order
                );
                app.store.ui.auto_sort_workspaces = false;
                refresh(&mut app, now);
                assert_eq!(order(&app), manual_order);
                app.store.ui.auto_sort_workspaces = true;
                refresh(&mut app, now);
                assert_eq!(order(&app)[0], recent_path);
                app.store.ui.auto_collapse_inactive_workspaces = true;
                refresh(&mut app, now - 1);
                assert!(!app.workspace_collapsed(&old_path));
                refresh(&mut app, now);
                assert!(app.workspace_collapsed(&old_path));
                assert!(app.workspace_collapsed(&empty_path));
                assert!(!app.workspace_collapsed(&recent_path));
                assert!(
                    !app.sidebar
                        .rows
                        .iter()
                        .any(|row| matches!(row, Row::Session { id: 1, .. }))
                );
                let sender = crate::app::queue::tests::running(&mut app, 1);
                refresh(&mut app, now);
                assert!(!app.workspace_collapsed(&old_path));
                assert_eq!(order(&app)[0], old_path);
                let recent_sender = crate::app::queue::tests::running(&mut app, 2);
                refresh(&mut app, now);
                assert_eq!(order(&app)[0], recent_path);
                app.session_control.runs.remove(&2);
                drop(recent_sender);
                app.session_control.runs.remove(&1);
                drop(sender);
                refresh(&mut app, now);
                assert!(app.workspace_collapsed(&old_path));
                assert_eq!(order(&app)[0], recent_path);
                app.set_workspace_collapsed(&old_path, false);
                refresh(&mut app, now + 1);
                assert!(!app.workspace_collapsed(&old_path));
                assert!(
                    app.sidebar
                        .rows
                        .iter()
                        .any(|row| matches!(row, Row::Session { id: 1, .. }))
                );
                app.set_workspace_collapsed(&recent_path, true);
                app.store.ui.auto_collapse_inactive_workspaces = false;
                refresh(&mut app, now + 2);
                assert!(!app.workspace_collapsed(&empty_path));
                assert!(app.workspace_collapsed(&recent_path));
                app.store.ui.auto_collapse_inactive_workspaces = true;
                app.selected_workspace = Some(2);
                refresh(&mut app, now + 3);
                assert!(!app.workspace_collapsed(&empty_path));
                app.selected_workspace = None;
                app.search_query = "search".into();
                refresh(&mut app, now + 4);
                assert!(app.sidebar.auto_collapsed.is_empty());
                app.search_query.clear();
                app.sidebar.auto_expanded.clear();
                app.session_mut(1).unwrap().created_at = now + 4;
                refresh(&mut app, now + 5);
                assert!(!app.workspace_collapsed(&old_path));
                assert!(app.workspace_collapsed(&empty_path));
                assert_eq!(order(&app)[0], old_path);
                // Equal activity retains the user's stored order, including cold metadata.
                app.session_mut(1).unwrap().created_at = now - 1;
                refresh(&mut app, now + 5);
                assert_eq!(order(&app), manual_order);
                for query in [
                    "非活跃",
                    "inactive workspace",
                    "活跃工作空间",
                    "sort active",
                ] {
                    assert!(
                        app.settings_search_matches(query)
                            .iter()
                            .any(|result| { result.page == 1 && result.group == Some(1) })
                    );
                }
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }
}
