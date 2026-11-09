//! Lightweight navigation metadata; transcripts stay in the store.
use super::*;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::menu::PopupMenu;
use std::rc::Rc;

const INACTIVE_WORKSPACE_AGE: u64 = 3 * 24 * 60 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Row {
    Controls,
    Workspace {
        path: PathBuf,
        count: usize,
        last: Option<u64>,
    },
    Loading(PathBuf),
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
            Self::Workspace { path, .. }
            | Self::Session { path, .. }
            | Self::More { path, .. }
            | Self::Loading(path) => Some(path),
            _ => None,
        }
    }
    pub(super) fn height(&self) -> f32 {
        match self {
            Self::Controls => ROW_HEIGHT,
            Self::More { .. } => 28.,
            Self::Empty => 76.,
            _ => ROW_HEIGHT,
        }
    }
    pub(super) fn same_identity(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Session { id: a, .. }, Self::Session { id: b, .. }) => a == b,
            _ => {
                std::mem::discriminant(self) == std::mem::discriminant(other)
                    && self.path() == other.path()
            }
        }
    }
}

#[derive(PartialEq, Eq)]
struct Summary {
    title: String,
    provider: Provider,
    native_id: Option<String>,
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
            archived: session.archived,
            pinned: session.pinned,
            activity: session.last_activity_at(),
            running: control.runs.contains_key(&session.id),
            has_user: session.has_user_messages(),
        }
    }
    fn sort_key(&self, id: u64) -> std::cmp::Reverse<(bool, bool, u64, u64)> {
        std::cmp::Reverse((self.pinned, self.running, self.activity, id))
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
    auto_expanded: HashSet<PathBuf>,
    auto_collapse_inactive: bool,
    loading: HashSet<PathBuf>,
    runs: HashSet<u64>,
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
            auto_expanded: app.sidebar.auto_expanded.clone(),
            auto_collapse_inactive: app.store.ui.auto_collapse_inactive_workspaces,
            loading: app.history_loading.clone(),
            runs: app.session_control.runs.keys().copied().collect(),
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
            && self.auto_expanded == other.auto_expanded
            && self.auto_collapse_inactive == other.auto_collapse_inactive
            && self.loading == other.loading
            && self.query == other.query
            && self.results == other.results
            && self.show_archived == other.show_archived
            && self.limit == other.limit
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
    indices: HashMap<PathBuf, usize>,
    inputs: Option<Inputs>,
    latest_activity: u64,
    origins: Vec<f32>,
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
            indices: HashMap::new(),
            inputs: None,
            latest_activity: 0,
            origins: Vec::new(),
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
        inputs: Inputs,
    ) -> bool {
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
                rebuild |=
                    order_changed || previous.is_none_or(|old| old.archived != next.archived);
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
                            && !group.summaries.values().any(|s| {
                                s.running
                                    || (s.activity > 0
                                        && now.saturating_sub(s.activity) < INACTIVE_WORKSPACE_AGE)
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
            let mut rows = vec![Row::Controls];
            for (index, workspace) in store.workspaces.iter().enumerate() {
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
                        (inputs.show_archived || !group.summaries[id].archived)
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
                        .or_else(|| workspace.sessions.first().map(|s| s.id)),
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
                for id in filtered.into_iter().take(if expanded {
                    total
                } else {
                    pinned + inputs.limit
                }) {
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
                origin += row.height() + 4.;
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
                });
            if let Some((index, offset)) = next {
                self.scroll
                    .set_offset(point(px(0.), px(-origins[index] - offset)));
            }
        }
        // Reuse the allocation when only identities change; layout sizes remain valid.
        let sizes: Vec<_> = rows
            .iter()
            .map(|row| size(px(0.), px(row.height())))
            .collect();
        if self.sizes.as_ref() != &sizes {
            self.sizes = Rc::new(sizes);
        }
        self.origins = origins;
        self.rows = Rc::new(rows);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{INACTIVE_WORKSPACE_AGE, Inputs, Row};
    use crate::app::{ConduitApp, Store, Workspace};
    use gpui_kit::{AppContext, TestAppContext};

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
                app.session_control.runs.remove(&1);
                drop(sender);
                refresh(&mut app, now);
                assert!(app.workspace_collapsed(&old_path));
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
                for query in ["非活跃", "inactive workspace"] {
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
