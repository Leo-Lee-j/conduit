//! Contextual file navigation and mixed chat/file documents.
use super::file_preview::FilePreview;
use super::*;
use crate::workspace_files::{self, Entry, FileKey, Stamp};
use conduit::plugins::{builtins, ids::ViewId};
use std::ops::Range;

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "mdx"]
                .iter()
                .any(|kind| extension.eq_ignore_ascii_case(kind))
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentTab {
    Settings,
    Usage(u64),
    PullRequest(u64),
    Chat(u64),
    File(u64),
    Diff(u64),
}
impl DocumentTab {
    pub(super) fn chat(self) -> Option<u64> {
        if let Self::Chat(id) = self {
            Some(id)
        } else {
            None
        }
    }
}
#[derive(Default)]
struct Directory {
    entries: Vec<Entry>,
    stamp: Option<Stamp>,
    loading: bool,
    error: Option<String>,
    sequence: u64,
}
#[derive(Default)]
struct Explorer {
    directories: HashMap<PathBuf, Directory>,
    expanded: HashSet<PathBuf>,
    show_ignored: bool,
    cached_rows: Option<Arc<Vec<TreeRow>>>,
    cached_compact_middle_packages: Option<bool>,
    /// Directories consulted for visible rows, including hidden middle segments.
    visible_directories: HashSet<PathBuf>,
}
struct Document {
    key: FileKey,
    is_image: bool,
    text: SharedString,
    image: Option<Arc<RenderImage>>,
    image_dimensions: Option<(u32, u32)>,
    highlights: Arc<Vec<(Range<usize>, &'static str)>>,
    stamp: Option<Stamp>,
    encoding: &'static str,
    loading: bool,
    error: Option<String>,
    revision: u64,
    sequence: u64,
    wrap: bool,
    preview: Entity<FilePreview>,
}
enum LoadedDocumentFile {
    Text {
        file: workspace_files::TextFile,
        highlights: Vec<(Range<usize>, &'static str)>,
    },
    Image {
        image: Arc<RenderImage>,
        dimensions: (u32, u32),
        stamp: Stamp,
    },
}
struct DiffDocument {
    workspace: PathBuf,
    file: git::ChangedFile,
    view: Entity<GitDiffDialog>,
}
#[derive(Default)]
pub(super) struct FileBrowser {
    tab_scroll: ScrollHandle,
    tab_layout: std::cell::RefCell<(Vec<DocumentTab>, Option<DocumentTab>, Pixels, bool)>,
    explorers: HashMap<PathBuf, Explorer>,
    documents: HashMap<u64, Document>,
    diffs: HashMap<u64, DiffDocument>,
    last_git_refresh: Option<Instant>,
    git_status: HashMap<PathBuf, TreeGitStatus>,
    preview: Option<u64>,
    next_id: u64,
    last_poll: Option<Instant>,
    polling: bool,
}
#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum TreeGitStatus {
    Untracked,
    Added,
    Deleted,
    Modified,
    Conflict,
}
impl TreeGitStatus {
    fn color(self) -> u32 {
        match self {
            Self::Untracked => palette().warning_text,
            Self::Added => palette().git_added_text,
            Self::Deleted => palette().git_deleted_text,
            Self::Modified => palette().git_modified_text,
            Self::Conflict => palette().error_text,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Untracked => "?",
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Modified => "M",
            Self::Conflict => "!",
        }
    }
}

#[derive(Clone)]
pub(super) struct FileDrag {
    pub(super) path: PathBuf,
}
impl Render for FileDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_2()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .bg(rgb(palette().surface_strong))
            .border_1()
            .border_color(rgb(palette().border))
            .text_color(rgb(palette().text))
            .text_size(ui_text_size(12.))
            .child(Icon::new(file_ref::icon_for(&self.path)).small())
            .child(
                self.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
    }
}

#[derive(Clone)]
enum TreeRow {
    Entry(Entry, usize, PathBuf),
    Status(PathBuf, usize, String),
}

fn grouped_tabs(
    tabs: &[DocumentTab],
    workspace: impl Fn(DocumentTab) -> Option<PathBuf>,
) -> Vec<DocumentTab> {
    let mut groups = HashMap::new();
    let mut ranked: Vec<_> = tabs
        .iter()
        .copied()
        .map(|tab| {
            let next = groups.len();
            let group = *groups.entry(workspace(tab)).or_insert(next);
            (group, tab)
        })
        .collect();
    ranked.sort_by_key(|(group, _)| *group);
    ranked.into_iter().map(|(_, tab)| tab).collect()
}

fn compact_middle_entry(
    explorer: &Explorer,
    entry: &Entry,
    visible: &mut HashSet<PathBuf>,
) -> Entry {
    let mut compacted = entry.clone();
    let mut names = vec![entry.name.clone()];
    let mut relative = entry.relative.clone();

    while compacted.directory && !compacted.link && relative.components().count() <= 64 {
        visible.insert(relative.clone());
        let Some(directory) = explorer.directories.get(&relative) else {
            break;
        };
        let Some(child) = directory.entries.first() else {
            break;
        };
        if directory.error.is_some()
            || directory.stamp.is_none()
            || directory.entries.len() != 1
            || !child.directory
            || child.link
            || child.relative.parent() != Some(relative.as_path())
        {
            break;
        }
        names.push(child.name.clone());
        relative = child.relative.clone();
        compacted = child.clone();
    }

    if names.len() > 1 {
        compacted.name = names.join("/");
    }
    compacted
}

fn flatten(
    explorer: &Explorer,
    path: &Path,
    depth: usize,
    compact_middle_packages: bool,
    rows: &mut Vec<TreeRow>,
    visible: &mut HashSet<PathBuf>,
) {
    visible.insert(path.to_owned());
    if path.components().count() > 64 {
        rows.push(TreeRow::Status(
            path.to_owned(),
            depth,
            conduit::i18n::text_static("files.directory_depth_exceeds_64_levels").into(),
        ));
        return;
    }
    let Some(directory) = explorer.directories.get(path) else {
        return;
    };
    if directory.loading && directory.entries.is_empty() {
        rows.push(TreeRow::Status(
            path.to_owned(),
            depth,
            conduit::i18n::text_static("dock.loading").into(),
        ));
    } else if let Some(error) = &directory.error {
        rows.push(TreeRow::Status(path.to_owned(), depth, error.clone()));
    } else if directory.entries.is_empty() {
        rows.push(TreeRow::Status(
            path.to_owned(),
            depth,
            conduit::i18n::text_static("files.no_visible_files").into(),
        ));
    }
    for entry in &directory.entries {
        let origin = entry.relative.clone();
        let entry = if compact_middle_packages {
            compact_middle_entry(explorer, entry, visible)
        } else {
            entry.clone()
        };
        rows.push(TreeRow::Entry(entry.clone(), depth, origin.clone()));
        if entry.directory && explorer.expanded.contains(&origin) {
            flatten(
                explorer,
                &entry.relative,
                depth + 1,
                compact_middle_packages,
                rows,
                visible,
            );
        }
    }
}

impl Explorer {
    fn rows(&mut self, compact_middle_packages: bool) -> Arc<Vec<TreeRow>> {
        if self.cached_compact_middle_packages != Some(compact_middle_packages) {
            // A chain may have finished loading after its first segment was expanded.
            // Keep its contents visible when switching back to separate directory rows.
            if !compact_middle_packages && let Some(rows) = &self.cached_rows {
                for row in rows.iter() {
                    if let TreeRow::Entry(entry, _, origin) = row
                        && entry.directory
                        && self.expanded.contains(origin)
                    {
                        self.expanded.extend(
                            entry
                                .relative
                                .ancestors()
                                .take_while(|path| !path.as_os_str().is_empty())
                                .map(Path::to_path_buf),
                        );
                    }
                }
            }
            self.cached_rows = None;
            self.cached_compact_middle_packages = Some(compact_middle_packages);
        }
        if let Some(rows) = &self.cached_rows {
            rows.clone()
        } else {
            let mut rows = Vec::new();
            let mut visible = HashSet::new();
            flatten(
                self,
                Path::new(""),
                0,
                compact_middle_packages,
                &mut rows,
                &mut visible,
            );
            self.visible_directories = visible;
            let rows = Arc::new(rows);
            self.cached_rows = Some(rows.clone());
            rows
        }
    }

    fn toggle(&mut self, origin: &Path, endpoint: &Path) -> bool {
        self.cached_rows = None;
        if self.expanded.remove(origin) {
            false
        } else {
            self.expanded.extend(
                endpoint
                    .ancestors()
                    .take_while(|p| !p.as_os_str().is_empty())
                    .map(Path::to_path_buf),
            );
            true
        }
    }
}

/// Streaming chat invalidations must not reshape a whole source document.
#[derive(Eq, PartialEq)]
struct FileViewSnapshot {
    document: Option<(u64, u64, u64, bool, Option<String>, bool)>,
    session: Option<u64>,
    approvals: Vec<(u64, usize)>,
    errors: Vec<u64>,
    appearance: (u32, u32, u32, Option<String>, u32, bool),
}
impl FileViewSnapshot {
    fn new(app: &ConduitApp) -> Self {
        let mut approvals: Vec<_> = app
            .session_control
            .runs
            .iter()
            .filter(|(_, r)| !r.approvals.is_empty())
            .map(|(id, r)| (*id, r.approvals.len()))
            .collect();
        approvals.sort_unstable();
        let mut errors: Vec<_> = app
            .session_control
            .unread_completed
            .iter()
            .copied()
            .filter(|id| {
                app.session(*id)
                    .is_some_and(|s| s.messages.last().is_some_and(|m| m.kind == ItemKind::Error))
            })
            .collect();
        errors.sort_unstable();
        Self {
            document: app.active_file.and_then(|id| {
                app.files.documents.get(&id).map(|d| {
                    (
                        id,
                        d.revision,
                        d.sequence,
                        d.loading,
                        d.error.clone(),
                        d.wrap,
                    )
                })
            }),
            session: app.selected_session,
            approvals,
            errors,
            appearance: (
                palette().canvas,
                palette().text,
                palette().muted,
                app.store.ui.code_font_family.clone(),
                app.store.ui.code_font_size.to_bits(),
                theme::background_visible(),
            ),
        }
    }
}
pub(super) struct FileDocumentView {
    app: WeakEntity<ConduitApp>,
    snapshot: Option<FileViewSnapshot>,
    #[cfg(test)]
    pub(super) render_count: usize,
    _subscription: Subscription,
}
impl FileDocumentView {
    pub(super) fn new(app: WeakEntity<ConduitApp>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&app.upgrade().expect("app is alive"), |view, app, cx| {
            let next = FileViewSnapshot::new(app.read(cx));
            if view.snapshot.as_ref() != Some(&next) {
                view.snapshot = Some(next);
                cx.notify();
            }
        });
        Self {
            app,
            snapshot: None,
            #[cfg(test)]
            render_count: 0,
            _subscription: subscription,
        }
    }
}
impl Render for FileDocumentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(test)]
        {
            self.render_count += 1;
        }
        if let Some(app) = self.app.upgrade() {
            self.snapshot = Some(FileViewSnapshot::new(app.read(cx)));
        }
        self.app
            .update(cx, |app, cx| app.render_file_document(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl ConduitApp {
    pub(super) fn active_document(&self) -> Option<DocumentTab> {
        self.settings_ui
            .open
            .then_some(DocumentTab::Settings)
            .or_else(|| {
                self.usage_ui
                    .active
                    .map(DocumentTab::Usage)
                    .or_else(|| self.github_ui.active.map(DocumentTab::PullRequest))
                    .or_else(|| self.active_diff.map(DocumentTab::Diff))
                    .or_else(|| self.active_file.map(DocumentTab::File))
                    .or_else(|| self.selected_session.map(DocumentTab::Chat))
            })
    }

    pub(super) fn select_document(
        &mut self,
        tab: DocumentTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_branch_navigation();
        if !matches!(tab, DocumentTab::Settings) {
            self.usage_ui.active = None;
            self.github_ui.active = None;
        }
        if !matches!(tab, DocumentTab::Settings) && self.settings_ui.open {
            self.settings_ui.open = false;
            self.settings_ui.focus_before_open = None;
            self.extensions_ui.probe = None;
            self.extensions_ui.cancel_mcp_detection();
            self.settings_ui
                .search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        match tab {
            DocumentTab::PullRequest(id) => {
                let Some(panel) = self.github_ui.panels.get(&id).cloned() else {
                    return;
                };
                let root = panel.read(cx).workspace.clone();
                let Some(index) = self.store.workspaces.iter().position(|w| w.path == root) else {
                    return;
                };
                if self.selected_workspace != Some(index) {
                    let session = self.store.workspaces[index]
                        .sessions
                        .iter()
                        .find(|s| !s.archived && !s.is_subagent)
                        .map(|s| s.id);
                    self.switch_context(index, session, false, window, cx);
                }
                self.active_file = None;
                self.active_diff = None;
                self.github_ui.active = Some(id);
                panel.read(cx).focus.clone().focus(window, cx);
                cx.notify();
            }
            DocumentTab::Usage(id) => {
                let Some(panel) = self.usage_ui.panels.get(&id).cloned() else {
                    return;
                };
                let root = panel.read(cx).workspace.clone();
                let Some(index) = self.store.workspaces.iter().position(|w| w.path == root) else {
                    return;
                };
                if self.selected_workspace != Some(index) {
                    let session = self.store.workspaces[index]
                        .sessions
                        .iter()
                        .find(|s| !s.archived && !s.is_subagent)
                        .map(|s| s.id);
                    self.switch_context(index, session, false, window, cx);
                }
                self.active_file = None;
                self.active_diff = None;
                self.usage_ui.active = Some(id);
                panel.update(cx, |panel, cx| panel.focus(window, cx));
                cx.notify();
            }
            DocumentTab::Settings => self.open_settings(window, cx),
            DocumentTab::Chat(id) => {
                self.active_diff = None;
                if let Some(index) = self
                    .store
                    .workspaces
                    .iter()
                    .position(|w| w.visible_sessions().any(|s| s.id == id))
                {
                    if self.selected_session == Some(id) && self.selected_workspace == Some(index) {
                        self.active_file = None;
                        self.push_tab(id);
                        self.session_control.unread_completed.remove(&id);
                        if let Some(provider) = self.session(id).map(|s| s.provider) {
                            self.new_session_provider = provider;
                            self.remember_provider(provider);
                        }
                        self.input.read(cx).focus_handle(cx).focus(window, cx);
                        cx.notify();
                    } else {
                        self.switch(index, Some(id), window, cx);
                    }
                }
            }
            DocumentTab::File(id) => {
                self.active_diff = None;
                let Some(key) = self.files.documents.get(&id).map(|d| d.key.clone()) else {
                    return;
                };
                let index = self
                    .store
                    .workspaces
                    .iter()
                    .position(|w| w.path == key.workspace);
                // Switch context only when necessary, preserving drafts and chat scroll on same-workspace browsing.
                if let Some(index) = index
                    && self.selected_workspace != Some(index)
                {
                    let session = self
                        .open_tabs
                        .iter()
                        .rev()
                        .filter_map(|t| t.chat())
                        .find(|id| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .any(|s| s.id == *id && !s.archived && !s.is_subagent)
                        })
                        .or_else(|| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .find(|s| !s.archived && !s.is_subagent)
                                .map(|s| s.id)
                        });
                    self.switch_context(index, session, false, window, cx);
                }
                self.active_file = Some(id);
                self.files.documents[&id]
                    .preview
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
                cx.notify();
            }
            DocumentTab::Diff(id) => {
                let Some((root, view)) = self
                    .files
                    .diffs
                    .get(&id)
                    .map(|d| (d.workspace.clone(), d.view.clone()))
                else {
                    return;
                };
                let Some(index) = self.store.workspaces.iter().position(|w| w.path == root) else {
                    return;
                };
                if self.selected_workspace != Some(index) {
                    let session = self
                        .open_tabs
                        .iter()
                        .rev()
                        .filter_map(|t| t.chat())
                        .find(|id| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .any(|s| s.id == *id && !s.archived && !s.is_subagent)
                        })
                        .or_else(|| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .find(|s| !s.archived && !s.is_subagent)
                                .map(|s| s.id)
                        });
                    self.switch_context(index, session, false, window, cx);
                }
                self.active_file = None;
                self.active_diff = Some(id);
                if self.git.snapshot.is_none() && self.git.probe.is_none() {
                    self.refresh_git(cx);
                }
                view.update(cx, |view, cx| {
                    if !view.loading {
                        view.reload(window, cx);
                    }
                    view.focus_diff(window, cx);
                });
                cx.notify();
            }
        }
    }

    pub(super) fn close_documents(
        &mut self,
        tabs: &[DocumentTab],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_document();
        let closing_active = active.is_some_and(|tab| tabs.contains(&tab));
        // Capture the owner before removing the document. Tab order is global,
        // so its last entry can belong to an unrelated workspace.
        let document_workspace = active
            .and_then(|tab| match tab {
                DocumentTab::PullRequest(id) => self.github_ui.roots.get(&id),
                DocumentTab::Usage(id) => self.usage_ui.roots.get(&id),
                DocumentTab::File(id) => self.files.documents.get(&id).map(|d| &d.key.workspace),
                DocumentTab::Diff(id) => self.files.diffs.get(&id).map(|d| &d.workspace),
                _ => None,
            })
            .and_then(|path| {
                self.store
                    .workspaces
                    .iter()
                    .position(|w| &w.path == path)
                    .or(self.selected_workspace)
            });
        let closing_settings = tabs.contains(&DocumentTab::Settings);
        if closing_settings {
            self.settings_ui.open = false;
            self.settings_ui.focus_before_open = None;
            self.extensions_ui.probe = None;
            self.extensions_ui.cancel_mcp_detection();
            self.settings_ui
                .search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        if closing_active && let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        }
        self.open_tabs.retain(|tab| !tabs.contains(tab));
        for tab in tabs {
            if let DocumentTab::PullRequest(id) = tab {
                self.github_ui.panels.remove(id);
                self.github_ui.roots.remove(id);
                if self.github_ui.active == Some(*id) {
                    self.github_ui.active = None;
                }
            }
            if let DocumentTab::Usage(id) = tab {
                self.usage_ui.panels.remove(id);
                self.usage_ui.roots.remove(id);
                if self.usage_ui.active == Some(*id) {
                    self.usage_ui.active = None;
                }
            }
            if let DocumentTab::Diff(id) = tab {
                self.files.diffs.remove(id);
            }
            if let DocumentTab::File(id) = tab {
                if let Some(document) = self.files.documents.remove(id)
                    && let Some(image) = document.image
                {
                    cx.drop_image(image, None);
                }
                if self.files.preview == Some(*id) {
                    self.files.preview = None;
                }
            }
        }
        if closing_active && let Some(workspace) = document_workspace {
            self.active_diff = None;
            self.active_file = None;
            let sessions = &self.store.workspaces[workspace].sessions;
            let available = |id: &u64| {
                !tabs.contains(&DocumentTab::Chat(*id))
                    && sessions
                        .iter()
                        .any(|s| s.id == *id && !s.archived && !s.is_subagent)
            };
            let session = self
                .selected_session
                .filter(available)
                .or_else(|| {
                    self.open_tabs
                        .iter()
                        .rev()
                        .filter_map(|tab| tab.chat())
                        .find(available)
                })
                .or_else(|| {
                    // Batch-close must not reopen a conversation the user closed.
                    if tabs.iter().any(|tab| tab.chat().is_some()) {
                        return None;
                    }
                    sessions
                        .iter()
                        .filter(|s| available(&s.id))
                        .max_by_key(|s| (s.last_activity_at(), s.id))
                        .map(|s| s.id)
                });
            if let Some(id) = session {
                self.select_document(DocumentTab::Chat(id), window, cx);
            } else {
                self.switch(workspace, None, window, cx);
            }
        } else if closing_active && !closing_settings {
            self.active_diff = None;
            self.active_file = None;
            if let Some(next) = self.open_tabs.last().copied() {
                self.select_document(next, window, cx);
            } else if let Some(workspace) = self.selected_workspace {
                self.switch(workspace, None, window, cx);
            } else {
                self.clear_workspace_selection(window, cx);
            }
        } else if closing_settings {
            let next = self
                .active_document()
                .filter(|tab| self.open_tabs.contains(tab))
                .or_else(|| self.open_tabs.last().copied());
            if let Some(active) = next {
                self.select_document(active, window, cx);
            } else if tabs.iter().any(|tab| *tab != DocumentTab::Settings) {
                if let Some(workspace) = self.selected_workspace {
                    self.switch(workspace, None, window, cx);
                } else {
                    self.clear_workspace_selection(window, cx);
                }
            } else {
                self.input.read(cx).focus_handle(cx).focus(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn forget_workspace_files(&mut self, root: &Path) {
        self.github_ui.sidebars.remove(root);
        let github_ids: HashSet<_> = self
            .github_ui
            .roots
            .iter()
            .filter(|(_, path)| path.as_path() == root)
            .map(|(id, _)| *id)
            .collect();
        self.open_tabs
            .retain(|tab| !matches!(tab, DocumentTab::PullRequest(id) if github_ids.contains(id)));
        self.github_ui
            .panels
            .retain(|id, _| !github_ids.contains(id));
        self.github_ui
            .roots
            .retain(|id, _| !github_ids.contains(id));
        if self
            .github_ui
            .active
            .is_some_and(|id| github_ids.contains(&id))
        {
            self.github_ui.active = None;
        }
        let usage_ids: HashSet<_> = self
            .usage_ui
            .roots
            .iter()
            .filter(|(_, path)| path.as_path() == root)
            .map(|(id, _)| *id)
            .collect();
        self.open_tabs
            .retain(|tab| !matches!(tab, DocumentTab::Usage(id) if usage_ids.contains(id)));
        self.usage_ui.panels.retain(|id, _| !usage_ids.contains(id));
        self.usage_ui.roots.retain(|id, _| !usage_ids.contains(id));
        if self
            .usage_ui
            .active
            .is_some_and(|id| usage_ids.contains(&id))
        {
            self.usage_ui.active = None;
        }
        let diff_ids: HashSet<_> = self
            .files
            .diffs
            .iter()
            .filter(|(_, d)| d.workspace == root)
            .map(|(id, _)| *id)
            .collect();
        self.open_tabs
            .retain(|t| !matches!(t, DocumentTab::Diff(id) if diff_ids.contains(id)));
        self.files.diffs.retain(|id, _| !diff_ids.contains(id));
        if self.active_diff.is_some_and(|id| diff_ids.contains(&id)) {
            self.active_diff = None;
        }
        let ids: HashSet<u64> = self
            .files
            .documents
            .iter()
            .filter(|(_, d)| d.key.workspace == root)
            .map(|(id, _)| *id)
            .collect();
        self.open_tabs
            .retain(|tab| !matches!(tab, DocumentTab::File(id) if ids.contains(id)));
        self.files.documents.retain(|id, _| !ids.contains(id));
        self.files.explorers.remove(root);
        if self.active_file.is_some_and(|id| ids.contains(&id)) {
            self.active_file = None;
        }
        if self.files.preview.is_some_and(|id| ids.contains(&id)) {
            self.files.preview = None;
        }
    }

    pub(super) fn open_workspace_file(
        &mut self,
        key: FileKey,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .store
            .workspaces
            .iter()
            .any(|w| w.path == key.workspace)
        {
            return;
        }
        self.open_file_preview(key, pinned, window, cx);
    }

    pub(super) fn current_file_key(&self) -> Option<FileKey> {
        self.active_file
            .and_then(|id| self.files.documents.get(&id))
            .map(|d| d.key.clone())
    }

    pub(super) fn reveal_file_position(
        &self,
        position: super::file_preview::SourcePosition,
        cx: &mut Context<Self>,
    ) {
        if let Some(document) = self
            .active_file
            .and_then(|id| self.files.documents.get(&id))
        {
            document
                .preview
                .update(cx, |preview, cx| preview.go_to(position, cx));
        }
    }

    pub(super) fn open_file_preview(
        &mut self,
        key: FileKey,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self
            .files
            .documents
            .iter()
            .find(|(_, d)| d.key == key)
            .map(|(id, _)| *id)
        {
            if pinned && self.files.preview == Some(id) {
                self.files.preview = None;
            }
            self.select_document(DocumentTab::File(id), window, cx);
            return;
        }
        let mut preview_position = None;
        if !pinned && let Some(old) = self.files.preview.take() {
            preview_position = self
                .open_tabs
                .iter()
                .position(|t| *t == DocumentTab::File(old));
            self.open_tabs.retain(|t| *t != DocumentTab::File(old));
            if let Some(document) = self.files.documents.remove(&old)
                && let Some(image) = document.image
            {
                cx.drop_image(image, None);
            }
        }
        self.files.next_id += 1;
        let id = self.files.next_id;
        let scroll = gpui_kit::ScrollHandle::new();
        let markdown = is_markdown(&key.relative);
        let is_image = workspace_files::is_image(&key.relative);
        let preview = cx.new(|cx| FilePreview::new(markdown, scroll.clone(), window, cx));
        self.files.documents.insert(
            id,
            Document {
                key,
                is_image,
                text: "".into(),
                image: None,
                image_dimensions: None,
                highlights: Arc::new(Vec::new()),
                stamp: None,
                encoding: "",
                loading: false,
                error: None,
                revision: 0,
                sequence: 0,
                wrap: false,
                preview,
            },
        );
        if let Some(position) = preview_position {
            self.open_tabs
                .insert(position.min(self.open_tabs.len()), DocumentTab::File(id));
        } else {
            self.open_tabs.push(DocumentTab::File(id));
        }
        if !pinned {
            self.files.preview = Some(id);
        }
        // Read-only files have a separate cap; browsing must never evict chat tabs.
        while self.files.documents.len() > 20 {
            let Some(old) = self.open_tabs.iter().find_map(|t| match t {
                DocumentTab::File(old) if *old != id => Some(*old),
                _ => None,
            }) else {
                break;
            };
            self.open_tabs.retain(|t| *t != DocumentTab::File(old));
            if let Some(document) = self.files.documents.remove(&old)
                && let Some(image) = document.image
            {
                cx.drop_image(image, None);
            }
            if self.files.preview == Some(old) {
                self.files.preview = None;
            }
        }
        self.select_document(DocumentTab::File(id), window, cx);
        self.load_file(id, cx);
    }

    fn load_file(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(document) = self.files.documents.get_mut(&id) else {
            return;
        };
        if document.loading {
            return;
        }
        if let Some(image) = document.image.take() {
            cx.drop_image(image, None);
        }
        document.image_dimensions = None;
        document.loading = true;
        document.sequence += 1;
        let sequence = document.sequence;
        let key = document.key.clone();
        let is_image = document.is_image;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    if is_image {
                        workspace_files::read_image(&key).map(|mut file| {
                            for pixel in file.pixels.pixels_mut() {
                                // GPUI's RenderImage consumes BGRA pixel data.
                                pixel.0.swap(0, 2);
                            }
                            LoadedDocumentFile::Image {
                                image: Arc::new(RenderImage::new(vec![image::Frame::new(
                                    file.pixels,
                                )])),
                                dimensions: (file.width, file.height),
                                stamp: file.stamp,
                            }
                        })
                    } else {
                        workspace_files::read(&key).map(|file| {
                            let highlights =
                                super::code_highlight::highlight_file(&key.relative, &file.text);
                            LoadedDocumentFile::Text { file, highlights }
                        })
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(document) = this.files.documents.get_mut(&id) else {
                    return;
                };
                if document.sequence != sequence {
                    return;
                }
                document.loading = false;
                match result {
                    Ok(LoadedDocumentFile::Text { file, highlights }) => {
                        if document.text.as_ref() != file.text {
                            document.revision += 1;
                        }
                        document.text = file.text.into();
                        document.image = None;
                        document.image_dimensions = None;
                        document.highlights = Arc::new(highlights);
                        document.stamp = Some(file.stamp);
                        document.encoding = file.encoding;
                        document.error = None;
                    }
                    Ok(LoadedDocumentFile::Image {
                        image,
                        dimensions,
                        stamp,
                    }) => {
                        document.revision += 1;
                        document.text = "".into();
                        document.image = Some(image);
                        document.image_dimensions = Some(dimensions);
                        document.highlights = Arc::default();
                        document.stamp = Some(stamp);
                        document.encoding = "Image";
                        document.error = None;
                    }
                    Err(error) => {
                        document.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_directory(&mut self, root: PathBuf, relative: PathBuf, cx: &mut Context<Self>) {
        let explorer = self.files.explorers.entry(root.clone()).or_default();
        explorer.cached_rows = None;
        let directory = explorer.directories.entry(relative.clone()).or_default();
        if directory.loading {
            return;
        }
        directory.loading = true;
        directory.sequence += 1;
        let sequence = directory.sequence;
        let show_ignored = explorer.show_ignored;
        let key = FileKey {
            workspace: root.clone(),
            relative: relative.clone(),
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { workspace_files::directory(&key, show_ignored) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(explorer) = this.files.explorers.get_mut(&root) else {
                    return;
                };
                if explorer.show_ignored != show_ignored {
                    return;
                }
                let Some(directory) = explorer.directories.get_mut(&relative) else {
                    return;
                };
                if directory.sequence != sequence {
                    return;
                }
                directory.loading = false;
                explorer.cached_rows = None;
                match result {
                    Ok((entries, stamp)) => {
                        directory.entries = entries;
                        directory.stamp = Some(stamp);
                        directory.error = None;
                    }
                    Err(error) => {
                        directory.entries.clear();
                        directory.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_tree(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let paths: Vec<_> = self
            .files
            .explorers
            .get(&root)
            .map(|e| {
                std::iter::once(PathBuf::new())
                    .chain(
                        e.visible_directories
                            .iter()
                            .filter(|p| !p.as_os_str().is_empty())
                            .cloned(),
                    )
                    .collect()
            })
            .unwrap_or_else(|| vec![PathBuf::new()]);
        for path in paths {
            self.load_directory(root.clone(), path, cx);
        }
        if self.git.probe.is_none() && self.git.operation.is_none() {
            self.refresh_git(cx);
        }
        cx.notify();
    }

    /// Poll only the visible document/expanded directories, without reading files on the UI thread.
    pub(super) fn poll_files(&mut self, cx: &mut Context<Self>) {
        if self.files.polling
            || self
                .files
                .last_poll
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        self.files.last_poll = Some(Instant::now());
        let document = self.active_file.and_then(|id| {
            self.files
                .documents
                .get(&id)
                .filter(|d| !d.loading && d.error.is_none())
                .map(|d| (id, d.key.clone(), d.stamp.clone()))
        });
        let root = (self.plugins.is_selected(builtins::FILES))
            .then(|| {
                self.selected_workspace
                    .map(|i| self.store.workspaces[i].path.clone())
            })
            .flatten();
        let directories: Vec<_> = root
            .as_ref()
            .and_then(|root| self.files.explorers.get(root).map(|e| (root, e)))
            .map(|(root, e)| {
                e.directories
                    .iter()
                    .filter(|(p, d)| {
                        (p.as_os_str().is_empty() || e.visible_directories.contains(*p))
                            && !d.loading
                            && d.error.is_none()
                    })
                    .map(|(p, d)| (root.clone(), p.clone(), d.stamp.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if document.is_none() && directories.is_empty() {
            return;
        }
        self.files.polling = true;
        cx.spawn(async move |this, cx| {
            let (changed_file, changed_dirs) = cx
                .background_executor()
                .spawn(async move {
                    let file = document.and_then(|(id, key, old)| {
                        (workspace_files::stamp(&key.workspace.join(&key.relative)).ok() != old)
                            .then_some(id)
                    });
                    let dirs: Vec<_> = directories
                        .into_iter()
                        .filter_map(|(root, path, old)| {
                            (workspace_files::stamp(&root.join(&path)).ok() != old)
                                .then_some((root, path))
                        })
                        .collect();
                    (file, dirs)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.files.polling = false;
                if let Some(id) = changed_file {
                    this.load_file(id, cx);
                }
                for (root, path) in changed_dirs {
                    if this.files.explorers.contains_key(&root) {
                        this.load_directory(root, path, cx);
                    }
                }
            });
        })
        .detach();
    }

    pub(super) fn activate_file_navigation(&mut self, cx: &mut Context<Self>) {
        if let Some(root) = self.selected_workspace_path() {
            self.refresh_tree(root, cx);
        }
    }

    pub(super) fn render_file_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.selected_workspace else {
            return sidebar::navigation_pane()
                .px_2()
                .py_3()
                .gap_3()
                .child(conduit::i18n::text_static("files.no_workspace_selected"))
                .child(
                    Button::new("files-pick-workspace")
                        .label(conduit::i18n::text_static("files.view_workspace"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.open_navigation(&ViewId::new(builtins::WORKSPACES).unwrap(), cx);
                        })),
                )
                .into_any_element();
        };
        let root = self.store.workspaces[index].path.clone();
        let name = self.store.workspaces[index].name();
        if !self
            .files
            .explorers
            .get(&root)
            .is_some_and(|e| e.directories.contains_key(Path::new("")))
        {
            self.load_directory(root.clone(), PathBuf::new(), cx);
        }
        let explorer = self.files.explorers.get_mut(&root).expect("created above");
        let show_ignored = explorer.show_ignored;
        let compact_middle_packages = self.store.ui.compact_middle_packages;
        let rows = explorer.rows(compact_middle_packages);
        // Probe only directories needed by the visible tree, with bounded concurrency.
        let loading = explorer.directories.values().filter(|d| d.loading).count();
        let mut pending: Vec<_> = explorer
            .visible_directories
            .iter()
            .filter(|path| path.components().count() <= 64)
            .filter(|path| {
                explorer
                    .directories
                    .get(*path)
                    .is_none_or(|d| !d.loading && d.stamp.is_none() && d.error.is_none())
            })
            .cloned()
            .collect();
        pending.sort();
        for path in pending.into_iter().take(8usize.saturating_sub(loading)) {
            self.load_directory(root.clone(), path, cx);
        }
        let app = cx.entity();
        let header = self.navigation_workspace_picker("files-workspace", name, root.clone(), cx);
        let options_app = cx.entity();
        let refresh_root = root.clone();
        let ignore_root = root.clone();
        let list_root = root.clone();
        let list = uniform_list(
            SharedString::from(format!("file-tree-{}", root.display())),
            rows.len(),
            move |range, _, cx| {
                app.update(cx, |this, cx| {
                    range
                        .map(|ordinal| match &rows[ordinal] {
                            TreeRow::Status(path, depth, message) => {
                                let root = list_root.clone();
                                let path = path.clone();
                                div()
                                    .id(SharedString::from(format!(
                                        "tree-status-{}",
                                        path.display()
                                    )))
                                    .h(px(28.))
                                    .pl(px(8. + *depth as f32 * 12.))
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .truncate()
                                    .child(message.clone())
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.load_directory(root.clone(), path.clone(), cx);
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            }
                            TreeRow::Entry(entry, depth, origin) => {
                                let origin = origin.clone();
                                let key = FileKey {
                                    workspace: list_root.clone(),
                                    relative: entry.relative.clone(),
                                };
                                let click_key = key.clone();
                                let pin_key = key.clone();
                                let drag_path = key.workspace.join(&key.relative);
                                let status = this.files.git_status.get(&drag_path).copied();
                                let directory = entry.directory;
                                let link = entry.link;
                                let expanded = this
                                    .files
                                    .explorers
                                    .get(&list_root)
                                    .is_some_and(|e| e.expanded.contains(&origin));
                                let active = this
                                    .active_file
                                    .and_then(|id| this.files.documents.get(&id))
                                    .is_some_and(|d| d.key == key);
                                h_flex()
                                    .id(SharedString::from(format!(
                                        "tree-entry-{}",
                                        entry.relative.display()
                                    )))
                                    .debug_selector({
                                        let path = entry.relative.to_string_lossy().into_owned();
                                        move || format!("file-tree-entry-{path}")
                                    })
                                    .h(px(28.))
                                    .min_w_0()
                                    .pl(px(8. + *depth as f32 * 12.))
                                    .pr_2()
                                    .rounded(px(8.))
                                    .gap_1()
                                    .items_center()
                                    .cursor_pointer()
                                    .when(active, |row| row.bg(rgb(palette().surface_hover)))
                                    .hover(|s| s.bg(rgb(palette().surface_hover)))
                                    .when_some(status, |row, status| {
                                        row.text_color(rgb(status.color()))
                                    })
                                    .when(!directory && this.writable && !this.closing, |row| {
                                        row.on_drag(
                                            FileDrag { path: drag_path },
                                            |drag, _, _, cx| {
                                                cx.stop_propagation();
                                                cx.new(|_| drag.clone())
                                            },
                                        )
                                    })
                                    .child(div().w(px(12.)).flex_shrink_0().when(
                                        directory,
                                        |slot| {
                                            slot.child(
                                                Icon::new(if expanded {
                                                    IconName::ChevronDown
                                                } else {
                                                    IconName::ChevronRight
                                                })
                                                .size(px(12.))
                                                .text_color(rgb(palette().muted)),
                                            )
                                        },
                                    ))
                                    .child(
                                        this.file_tree_icon(&entry.relative, directory, expanded)
                                            .small(),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .text_size(ui_text_size(12.))
                                            .child(format!(
                                                "{}{}",
                                                entry.name,
                                                if link { " ↗" } else { "" }
                                            )),
                                    )
                                    .when_some(status.filter(|_| !directory), |row, status| {
                                        row.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_size(ui_text_size(10.))
                                                .child(status.label()),
                                        )
                                    })
                                    .tooltip({
                                        let label = workspace_files::display_path(
                                            &key.workspace.join(&key.relative),
                                        );
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                label.clone(),
                                            )
                                            .build(window, cx)
                                        }
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if directory {
                                            if let Some(e) =
                                                this.files.explorers.get_mut(&click_key.workspace)
                                                && e.toggle(&origin, &click_key.relative)
                                            {
                                                this.load_directory(
                                                    click_key.workspace.clone(),
                                                    click_key.relative.clone(),
                                                    cx,
                                                );
                                            }
                                            cx.notify();
                                        } else {
                                            this.open_workspace_file(
                                                click_key.clone(),
                                                false,
                                                window,
                                                cx,
                                            );
                                        }
                                    }))
                                    .on_double_click(cx.listener(move |this, _, window, cx| {
                                        if !directory {
                                            this.open_workspace_file(
                                                pin_key.clone(),
                                                true,
                                                window,
                                                cx,
                                            );
                                        }
                                    }))
                                    .test_support()
                                    .into_any_element()
                            }
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .h_full()
        .w_full();
        sidebar::navigation_pane()
            .id("file-explorer")
            .debug_selector(|| "file-explorer".into())
            .child(
                sidebar::navigation_header()
                    .child(div().flex_1().min_w_0().overflow_hidden().child(header))
                    .child(
                        self.project_files_button(cx, None)
                            .debug_selector(|| "context-file-tools".into()),
                    )
                    .child(
                        Button::new("files-view-options")
                            .debug_selector(|| "files-view-options".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Settings)
                            .tooltip(conduit::i18n::text_static("files.view_options"))
                            .accessibility_label(conduit::i18n::text_static("files.view_options"))
                            .disabled(!self.writable || self.closing)
                            .dropdown_menu(move |menu, _, cx| {
                                let compact = options_app.read(cx).store.ui.compact_middle_packages;
                                let grouped = options_app.read(cx).store.ui.group_tabs;
                                let compact_app = options_app.clone();
                                let group_app = options_app.clone();
                                menu.item(
                                    PopupMenuItem::new(conduit::i18n::text_static(
                                        "settings.general.compact_middle_packages",
                                    ))
                                    .checked(compact)
                                    .on_click(
                                        move |_, _, cx| {
                                            compact_app.update(cx, |this, cx| {
                                                if !this.writable || this.closing {
                                                    return;
                                                }
                                                this.store.ui.compact_middle_packages =
                                                    !this.store.ui.compact_middle_packages;
                                                this.persist();
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(conduit::i18n::text_static(
                                        "settings.general.group_tabs",
                                    ))
                                    .checked(grouped)
                                    .on_click(
                                        move |_, _, cx| {
                                            group_app.update(cx, |this, cx| {
                                                if !this.writable || this.closing {
                                                    return;
                                                }
                                                this.store.ui.group_tabs =
                                                    !this.store.ui.group_tabs;
                                                this.persist();
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                            }),
                    )
                    .child(
                        Button::new("files-show-ignored")
                            .debug_selector(|| "files-show-ignored".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::Eye)
                            .tooltip(if show_ignored {
                                conduit::i18n::text_static("files.hide_ignored_files")
                            } else {
                                conduit::i18n::text_static("files.show_ignored_files")
                            })
                            .accessibility_label(if show_ignored {
                                conduit::i18n::text_static("files.hide_ignored_files")
                            } else {
                                conduit::i18n::text_static("files.show_ignored_files")
                            })
                            .selected(show_ignored)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(e) = this.files.explorers.get_mut(&ignore_root) {
                                    e.show_ignored = !e.show_ignored;
                                    for d in e.directories.values_mut() {
                                        d.loading = false;
                                        d.entries.clear();
                                        d.stamp = None;
                                        d.error = None;
                                        d.sequence += 1;
                                    }
                                }
                                this.refresh_tree(ignore_root.clone(), cx);
                            })),
                    )
                    .child(
                        Button::new("files-refresh")
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::RefreshCw)
                            .tooltip(conduit::i18n::text_static("files.refresh_file_tree"))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.refresh_tree(refresh_root.clone(), cx)
                            })),
                    ),
            )
            .child(sidebar::navigation_content().flex_1().min_h_0().child(list))
            .into_any_element()
    }

    pub(super) fn render_file_document(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = self.active_file else {
            return div().into_any_element();
        };
        let Some(document) = self.files.documents.get(&id) else {
            return div()
                .child(conduit::i18n::text_static("files.file_tab_closed"))
                .into_any_element();
        };
        let path = document.key.workspace.join(&document.key.relative);
        let path_label = workspace_files::display_path(&path);
        let path_to_copy = path_label.clone();
        let workspace_name = document
            .key
            .workspace
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| workspace_files::display_path(&document.key.workspace));
        let mut parts: Vec<_> = document
            .key
            .relative
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let file_name = parts.pop().unwrap_or_default();
        let markdown = is_markdown(&document.key.relative);
        let is_image = document.is_image;
        let parent_label = parts.join(" / ");
        let text = document.text.clone();
        let copy = text.clone();
        let wrap = document.wrap;
        let image = document.image.clone();
        let status = if document.loading {
            conduit::i18n::text_static("files.reading").to_string()
        } else if document.error.is_some() {
            conduit::i18n::text_static("files.read_only_preview_unavailable").to_string()
        } else if let Some((width, height)) = document.image_dimensions {
            conduit::i18n::format(
                "files.read_only_image",
                &[
                    ("arg0", width.to_string()),
                    ("arg1", height.to_string()),
                    (
                        "arg2",
                        format!(
                            "{} KiB",
                            document
                                .stamp
                                .as_ref()
                                .map_or(0, |stamp| stamp.len.div_ceil(1024))
                        ),
                    ),
                ],
            )
        } else {
            conduit::i18n::format(
                "files.read_only_lines",
                &[
                    ("arg0", document.encoding.to_string()),
                    ("arg1", format!("{}", text.lines().count())),
                ],
            )
        };
        // Cached views are laid out as roots: flex_1 alone is not a definite
        // viewport height and lets long files grow beyond the window.
        let mut page = v_flex()
            .id("file-document")
            .debug_selector(|| "file-document".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .font_family(Theme::global(cx).font_family.clone())
            .child(
                h_flex()
                    .id("file-document-toolbar")
                    .debug_selector(|| "file-document-toolbar".into())
                    .h(px(36.))
                    .flex_shrink_0()
                    .min_w_0()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new(("file-return-chat", id))
                            .debug_selector(|| "file-return-chat".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::ArrowLeft)
                            .tooltip(conduit::i18n::text_static("files.back_to_chat"))
                            .accessibility_label(conduit::i18n::text_static("files.back_to_chat"))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(id) = this.selected_session {
                                    this.select_document(DocumentTab::Chat(id), window, cx);
                                } else {
                                    this.active_file = None;
                                }
                                this.input.read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        h_flex()
                            .id("file-path-breadcrumb")
                            .debug_selector(|| "file-path-breadcrumb".into())
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .items_center()
                            .gap_1()
                            .px_1()
                            .text_size(ui_text_size(11.))
                            .tooltip({
                                let label = path_label.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(
                                div()
                                    .max_w(px(120.))
                                    .min_w_0()
                                    .truncate()
                                    .text_color(rgb(palette().muted))
                                    .child(workspace_name),
                            )
                            .child(
                                Icon::new(IconName::ChevronRight)
                                    .size(px(12.))
                                    .text_color(rgb(palette().faint)),
                            )
                            .when(!parent_label.is_empty(), |row| {
                                row.child(
                                    div()
                                        .max_w(relative(0.4))
                                        .min_w_0()
                                        .truncate()
                                        .text_color(rgb(palette().muted))
                                        .child(parent_label),
                                )
                                .child(
                                    Icon::new(IconName::ChevronRight)
                                        .size(px(12.))
                                        .text_color(rgb(palette().faint)),
                                )
                            })
                            .child(
                                div()
                                    .id("file-path-name")
                                    .debug_selector(|| "file-path-name".into())
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(file_name),
                            ),
                    )
                    .child(
                        Button::new(("file-wrap", id))
                            .debug_selector(|| "file-wrap".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::TextWrap)
                            .tooltip(if markdown {
                                conduit::i18n::text_static("files.markdown_previews_always_wrap")
                            } else if wrap {
                                conduit::i18n::text_static("files.word_wrap_on_click_to_disable")
                            } else {
                                conduit::i18n::text_static("files.enable_word_wrap")
                            })
                            .accessibility_label(conduit::i18n::text_static("files.word_wrap"))
                            .selected(wrap || markdown)
                            .disabled(markdown || is_image)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(d) = this.files.documents.get_mut(&id) {
                                    d.wrap = !d.wrap;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new(("file-reload", id))
                            .debug_selector(|| "file-reload".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::RefreshCw)
                            .tooltip(conduit::i18n::text_static("files.reload_file"))
                            .accessibility_label(conduit::i18n::text_static("files.reload_file"))
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| this.load_file(id, cx))),
                    )
                    .child(
                        Button::new(("file-copy-path-toolbar", id))
                            .debug_selector(|| "file-copy-path-toolbar".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(FullIconName::Link)
                            .tooltip(conduit::i18n::text_static("files.copy_file_path"))
                            .accessibility_label(conduit::i18n::text_static("files.copy_file_path"))
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    path_to_copy.clone(),
                                ))
                            }),
                    )
                    .child(
                        Button::new(("file-copy", id))
                            .debug_selector(|| "file-copy".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::Copy)
                            .tooltip(conduit::i18n::text_static("files.copy_file_contents"))
                            .accessibility_label(conduit::i18n::text_static(
                                "files.copy_file_contents",
                            ))
                            .cursor_pointer()
                            .disabled(document.error.is_some() || document.loading || is_image)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.to_string()))
                            }),
                    ),
            );
        if let Some(error) = &document.error {
            page = page.child(
                v_flex()
                    .flex_1()
                    .p_6()
                    .gap_3()
                    .child(div().font_semibold().child(conduit::i18n::text_static(
                        "files.unable_to_preview_this_file",
                    )))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette().muted))
                            .child(error.clone()),
                    )
                    .child(
                        Button::new(("file-copy-path", id))
                            .debug_selector(|| "file-copy-path".into())
                            .label(conduit::i18n::text_static("files.copy_file_path"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(path_label.clone()))
                            }),
                    ),
            );
        } else if document.loading && text.is_empty() {
            page = page.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(Spinner::new()),
            );
        } else if text.is_empty() {
            if let Some(image) = image {
                page = page.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .w_full()
                        .overflow_hidden()
                        .bg(rgb(palette().canvas))
                        .child(
                            gpui_kit::img(image)
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        ),
                );
            } else {
                page = page.child(
                    v_flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .child(conduit::i18n::text_static("files.empty_file")),
                );
            }
        } else {
            let mut typography = self.chat_typography();
            if typography.body_font.is_none() {
                typography.body_font = Some(Theme::global(cx).font_family.clone());
            }
            document.preview.update(cx, |preview, cx| {
                preview.sync(text, document.highlights.clone(), wrap, typography, cx);
            });
            page = page.child(document.preview.clone());
        }
        page = page.child(
            h_flex()
                .id("file-document-status")
                .debug_selector(|| "file-document-status".into())
                .h(px(24.))
                .flex_shrink_0()
                .min_w_0()
                .px_3()
                .items_center()
                .border_t_1()
                .border_color(rgb(palette().border))
                .text_size(ui_text_size(10.))
                .text_color(rgb(palette().muted))
                .child(div().min_w_0().truncate().child(status)),
        );
        if let Some(attention) = self.document_attention(cx) {
            page = page.child(attention);
        }
        page.into_any_element()
    }

    fn document_tabs(&self) -> Vec<DocumentTab> {
        let tabs = self.open_tabs.clone();
        if !self.store.ui.group_tabs {
            return tabs;
        }
        grouped_tabs(&tabs, |tab| self.tab_workspace(&tab))
    }

    fn tab_workspace(&self, tab: &DocumentTab) -> Option<PathBuf> {
        match *tab {
            DocumentTab::Settings => None,
            DocumentTab::PullRequest(id) => self.github_ui.roots.get(&id).cloned(),
            DocumentTab::Usage(id) => self.usage_ui.roots.get(&id).cloned(),
            DocumentTab::Chat(id) => self
                .store
                .workspaces
                .iter()
                .find(|workspace| workspace.sessions.iter().any(|session| session.id == id))
                .map(|workspace| workspace.path.clone()),
            DocumentTab::File(id) => self
                .files
                .documents
                .get(&id)
                .map(|document| document.key.workspace.clone()),
            DocumentTab::Diff(id) => self
                .files
                .diffs
                .get(&id)
                .map(|document| document.workspace.clone()),
        }
    }

    pub(super) fn document_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let tabs = self.document_tabs();
        let selected = self
            .active_document()
            .and_then(|t| tabs.iter().position(|item| *item == t));
        let active = self.active_document();
        {
            let mut layout = self.files.tab_layout.borrow_mut();
            if layout.0 != tabs || layout.1 != active || layout.3 != self.store.ui.group_tabs {
                layout.0 = tabs.clone();
                layout.1 = active;
                layout.3 = self.store.ui.group_tabs;
                if let Some(index) = selected {
                    self.files.tab_scroll.scroll_to_item(index);
                }
            }
        }
        let mut bar = TabBar::new("session-tabs")
            .folder()
            .menu(true)
            .max_width(px(240.))
            .track_scroll(&self.files.tab_scroll)
            .with_size(Size::Medium)
            .h(px(TAB_BAR_HEIGHT))
            .pr(px(8.))
            .when_some(selected, |bar, i| bar.selected_index(i));
        let mut previous_workspace = None;
        for tab in tabs.iter().copied() {
            let (title, tooltip, icon, running, unread, selector) = match tab {
                DocumentTab::PullRequest(id) => (
                    self.github_ui
                        .panels
                        .get(&id)
                        .map(|panel| panel.read(cx).tab_title())
                        .unwrap_or_else(|| conduit::i18n::text_static("github.title").to_owned()),
                    self.github_ui
                        .roots
                        .get(&id)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    Icon::new(FullIconName::GitPullRequest)
                        .small()
                        .into_any_element(),
                    false,
                    false,
                    format!("pr-tab-{id}"),
                ),
                DocumentTab::Usage(id) => (
                    conduit::i18n::text_static("usage.title").to_owned(),
                    self.usage_ui
                        .roots
                        .get(&id)
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    Icon::new(FullIconName::ChartColumn)
                        .small()
                        .into_any_element(),
                    false,
                    false,
                    format!("usage-tab-{id}"),
                ),
                DocumentTab::Settings => (
                    conduit::i18n::text_static("files.settings").to_owned(),
                    conduit::i18n::text_static("files.settings").to_owned(),
                    Icon::new(IconName::Settings).small().into_any_element(),
                    false,
                    false,
                    "settings-tab".to_owned(),
                ),
                DocumentTab::Chat(id) => {
                    let Some(s) = self.session(id) else {
                        continue;
                    };
                    let title = single_line_title(if s.title.is_empty() {
                        conduit::i18n::text_static("files.new_session")
                    } else {
                        &s.title
                    });
                    (
                        title.clone(),
                        title,
                        provider_icon(s.provider).into_any_element(),
                        self.session_control.runs.contains_key(&id),
                        self.session_control.unread_completed.contains(&id),
                        format!("session-tab-{id}"),
                    )
                }
                DocumentTab::File(id) => {
                    let Some(d) = self.files.documents.get(&id) else {
                        continue;
                    };
                    let name = d
                        .key
                        .relative
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let duplicate = self
                        .files
                        .documents
                        .values()
                        .filter(|other| {
                            other.key.relative.file_name() == d.key.relative.file_name()
                        })
                        .count()
                        > 1;
                    let title = if duplicate {
                        format!(
                            "{name} · {}/{}",
                            d.key
                                .workspace
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                            d.key.relative.parent().unwrap_or(Path::new("")).display()
                        )
                    } else {
                        name
                    };
                    (
                        title,
                        workspace_files::display_path(&d.key.workspace.join(&d.key.relative)),
                        Icon::new(file_ref::icon_for(&d.key.relative))
                            .small()
                            .into_any_element(),
                        false,
                        false,
                        format!("file-tab-{id}"),
                    )
                }
                DocumentTab::Diff(id) => {
                    let Some(d) = self.files.diffs.get(&id) else {
                        continue;
                    };
                    let title = format!(
                        "Diff · {}",
                        Path::new(&d.file.path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    );
                    (
                        title,
                        conduit::i18n::format(
                            "files.head_working_tree",
                            &[(
                                "arg0",
                                format!("{}", d.workspace.join(&d.file.path).display()),
                            )],
                        ),
                        Icon::new(gpui_kit::assets::IconName::FileDiff)
                            .small()
                            .into_any_element(),
                        false,
                        false,
                        format!("diff-tab-{id}"),
                    )
                }
            };
            let selector_close = match tab {
                DocumentTab::Settings => "settings-tab-close".to_owned(),
                DocumentTab::Usage(id) => format!("usage-tab-close-{id}"),
                DocumentTab::PullRequest(id) => format!("pr-tab-close-{id}"),
                DocumentTab::Chat(id) => format!("session-tab-close-{id}"),
                DocumentTab::File(id) => format!("file-tab-close-{id}"),
                DocumentTab::Diff(id) => format!("diff-tab-close-{id}"),
            };
            let workspace = self.tab_workspace(&tab);
            let starts_group =
                self.store.ui.group_tabs && workspace.is_some() && workspace != previous_workspace;
            previous_workspace = workspace.clone();
            let icon = if starts_group {
                let root = workspace.expect("group has a workspace");
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .id(SharedString::from(format!("tab-group-{}", root.display())))
                            .debug_selector({
                                let root = root.clone();
                                move || format!("tab-group-{}", root.display())
                            })
                            .max_w(px(90.))
                            .truncate()
                            .pr_2()
                            .border_r_1()
                            .border_color(rgb(palette().border))
                            .text_size(ui_text_size(10.))
                            .text_color(rgb(palette().muted))
                            .child(
                                root.file_name()
                                    .unwrap_or(root.as_os_str())
                                    .to_string_lossy()
                                    .into_owned(),
                            ),
                    )
                    .child(icon)
                    .into_any_element()
            } else {
                icon
            };
            let group = SharedString::from(selector.clone());
            let preview = matches!(tab, DocumentTab::File(id) if self.files.preview == Some(id));
            let mut item = Tab::new()
                .debug_selector(move || selector.clone())
                .aria_label(title.clone())
                .group(group.clone())
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
                })
                .on_hover(cx.listener(move |this, hovered, _, _| {
                    if *hovered {
                        this.tab_menu_target = Some(tab);
                    } else if this.tab_menu_target == Some(tab) {
                        this.tab_menu_target = None;
                    }
                }))
                .prefix(icon)
                .label(title)
                .text_size(ui_text_size(12.))
                .when(preview, |style| style.italic());
            if let DocumentTab::Chat(id) = tab
                && (running || unread)
            {
                item = item.child(session_activity_dot("tab-activity", id, running));
            }
            bar = bar.child(
                item.suffix(
                    Button::new(SharedString::from(selector_close.clone()))
                        .debug_selector(move || selector_close.clone())
                        .ghost()
                        .small()
                        .compact()
                        .icon(gpui_kit::assets::IconName::X)
                        .opacity(0.)
                        .group_hover(group, |s| s.opacity(1.))
                        .focus_visible(|s| s.opacity(1.))
                        .tooltip(conduit::i18n::text_static("files.close_tab"))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.close_documents(&[tab], window, cx);
                            cx.stop_propagation();
                        })),
                )
                .on_click(cx.listener(
                    move |this, event: &ClickEvent, window, cx| {
                        this.select_document(tab, window, cx);
                        if event.click_count() == 2
                            && let DocumentTab::File(id) = tab
                            && this.files.preview == Some(id)
                        {
                            this.files.preview = None;
                            cx.notify();
                        }
                    },
                )),
            );
        }
        if tabs.is_empty() {
            bar = bar.prefix(
                div()
                    .pl_3()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static("files.select_a_session_or_file")),
            );
        }
        let app = cx.entity();
        let resize_app = app.downgrade();
        div()
            .id("document-tab-strip")
            .test_support()
            .debug_selector(|| "document-tab-strip".into())
            .relative()
            .h(px(TAB_BAR_HEIGHT))
            .min_w_0()
            .flex_shrink_0()
            .on_prepaint(move |bounds, window, cx| {
                let _ = resize_app.update(cx, |this, cx| {
                    let mut layout = this.files.tab_layout.borrow_mut();
                    if layout.2 != bounds.size.width {
                        layout.2 = bounds.size.width;
                        // The tracked viewport still contains the previous
                        // frame's width here. Reveal after its layout updates.
                        let view = cx.weak_entity();
                        window.on_next_frame(move |_, cx| {
                            let _ = view.update(cx, |this, cx| {
                                if let Some(index) = this.active_document().and_then(|tab| {
                                    this.document_tabs().iter().position(|item| *item == tab)
                                }) {
                                    this.files.tab_scroll.scroll_to_item(index);
                                    cx.notify();
                                }
                            });
                        });
                    }
                });
            })
            .child(bar)
            .context_menu(move |menu, _, cx| {
                let Some(target) = app.read(cx).tab_menu_target else {
                    return menu;
                };
                let right = app.clone();
                let left = app.clone();
                let all = app.clone();
                let pin = app.clone();
                menu.when(matches!(target, DocumentTab::File(_)), |menu| {
                    menu.item(
                        PopupMenuItem::new(conduit::i18n::text_static("files.pin_file_tab"))
                            .on_click(move |_, _, cx| {
                                pin.update(cx, |this, cx| {
                                    if let DocumentTab::File(id) = target
                                        && this.files.preview == Some(id)
                                    {
                                        this.files.preview = None;
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                })
                .item(
                    PopupMenuItem::new(conduit::i18n::text_static("files.close_tabs_to_the_right"))
                        .on_click(move |_, window, cx| {
                            right.update(cx, |this, cx| {
                                let ordered = this.document_tabs();
                                if let Some(i) = ordered.iter().position(|t| *t == target) {
                                    let tabs = ordered[i + 1..].to_vec();
                                    this.close_documents(&tabs, window, cx);
                                }
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(conduit::i18n::text_static("files.close_tabs_to_the_left"))
                        .on_click(move |_, window, cx| {
                            left.update(cx, |this, cx| {
                                let ordered = this.document_tabs();
                                if let Some(i) = ordered.iter().position(|t| *t == target) {
                                    let tabs = ordered[..i].to_vec();
                                    this.close_documents(&tabs, window, cx);
                                }
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(conduit::i18n::text_static("files.close_all")).on_click(
                        move |_, window, cx| {
                            all.update(cx, |this, cx| {
                                let tabs = this.open_tabs.clone();
                                this.close_documents(&tabs, window, cx);
                            });
                        },
                    ),
                )
            })
            .into_any_element()
    }
}

impl ConduitApp {
    /// Git is navigation, not a second competing right-side detail dock.
    pub(super) fn git_navigation_open(&self) -> bool {
        self.plugins.is_selected(builtins::GIT) && !self.plugins.collapsed()
    }

    pub(super) fn git_status_visible(&self) -> bool {
        (!self.plugins.collapsed()
            && matches!(
                self.plugins.selected().as_str(),
                builtins::FILES | builtins::GIT
            ))
            || self.branch_navigation_open()
            || self.active_diff.is_some()
    }

    pub(super) fn clear_tree_git_status(&mut self) {
        self.files.git_status.clear();
    }

    pub(super) fn update_tree_git_status(&mut self, workspace: &Path, snapshot: &git::Snapshot) {
        self.clear_tree_git_status();
        let root = git::worktree_root(workspace);
        for file in &snapshot.files {
            let status = if file.index == 'U'
                || file.worktree == 'U'
                || matches!((file.index, file.worktree), ('A', 'A') | ('D', 'D'))
            {
                TreeGitStatus::Conflict
            } else if file.untracked() {
                TreeGitStatus::Untracked
            } else {
                match file.kind() {
                    git::ChangeKind::Added => TreeGitStatus::Added,
                    git::ChangeKind::Modified => TreeGitStatus::Modified,
                    git::ChangeKind::Deleted => TreeGitStatus::Deleted,
                }
            };
            let path = root.join(&file.path);
            for path in path
                .ancestors()
                .take_while(|path| *path != root && path.starts_with(&root))
            {
                self.files
                    .git_status
                    .entry(path.to_path_buf())
                    .and_modify(|current| *current = (*current).max(status))
                    .or_insert(status);
            }
        }
    }

    pub(super) fn mark_git_refresh(&mut self) {
        self.files.last_git_refresh = Some(Instant::now());
    }

    pub(super) fn poll_git_navigation(&mut self, cx: &mut Context<Self>) {
        if self.git_status_visible()
            && self.git.probe.is_none()
            && self.git.operation.is_none()
            && self.git.generation.is_none()
            && self
                .files
                .last_git_refresh
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(10))
        {
            self.refresh_git(cx);
        }
    }

    pub(super) fn leave_branch_navigation(&mut self) {
        self.branches_open = false;
    }

    pub(super) fn branch_navigation_open(&self) -> bool {
        self.branches_open
    }

    pub(super) fn show_branch_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch_manager.is_none() {
            let app = cx.weak_entity();
            let workspace = self.selected_workspace_path();
            let repository = self.git.repository.clone();
            self.branch_manager =
                Some(cx.new(|cx| git_log::GitLogView::new(app, workspace, repository, window, cx)));
        }
        if let Some(view) = &self.branch_manager {
            view.read(cx)
                .branch_search
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        }
        self.branches_open = true;
        self.refresh_branches();
        cx.notify();
    }

    pub(super) fn toggle_git_navigation(&mut self, cx: &mut Context<Self>) {
        if self.git_navigation_open() {
            self.open_navigation(&ViewId::new(builtins::WORKSPACES).unwrap(), cx);
        } else {
            self.show_git_navigation(cx);
        }
    }

    pub(super) fn show_git_navigation(&mut self, cx: &mut Context<Self>) {
        self.open_navigation(&ViewId::new(builtins::GIT).unwrap(), cx);
    }

    pub(super) fn navigation_workspace_picker(
        &self,
        id: &'static str,
        name: String,
        root: PathBuf,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let app = cx.entity();
        let choices: Vec<_> = self
            .store
            .workspaces
            .iter()
            .map(|w| (w.path.clone(), w.name()))
            .collect();
        Button::new(id)
            .debug_selector(move || id.into())
            .ghost()
            .small()
            .w_full()
            .min_w_0()
            .accessibility_label(name.clone())
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .items_center()
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(Icon::new(IconName::ChevronDown).small().flex_shrink_0()),
            )
            .tooltip(display_path(&root))
            .cursor_pointer()
            .dropdown_menu(move |mut menu, _, _| {
                for (path, name) in &choices {
                    let path = path.clone();
                    let app = app.clone();
                    menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                if let Some(i) =
                                    this.store.workspaces.iter().position(|w| w.path == path)
                                {
                                    let session = this
                                        .open_tabs
                                        .iter()
                                        .rev()
                                        .filter_map(|t| t.chat())
                                        .find(|id| {
                                            this.store.workspaces[i].sessions.iter().any(|s| {
                                                s.id == *id && !s.archived && !s.is_subagent
                                            })
                                        })
                                        .or_else(|| {
                                            this.store.workspaces[i]
                                                .sessions
                                                .iter()
                                                .find(|s| !s.archived && !s.is_subagent)
                                                .map(|s| s.id)
                                        });
                                    let navigation = this.plugins.selected().clone();
                                    if let Some(id) = session {
                                        this.select_document(DocumentTab::Chat(id), window, cx);
                                    } else {
                                        this.switch(i, None, window, cx);
                                    }
                                    this.open_navigation(&navigation, cx);
                                }
                            });
                        },
                    ));
                }
                menu
            })
            .into_any_element()
    }

    pub(super) fn render_git_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity();
        let mut pane = sidebar::navigation_pane()
            .id("git-navigation")
            .test_support()
            .debug_selector(|| "git-navigation".into());
        let requests_active = self.github_ui.sidebar_active;
        let request_content = requests_active.then(|| self.request_sidebar(cx));
        let request_sidebar = self
            .selected_workspace_path()
            .and_then(|path| self.github_ui.sidebars.get(&path).cloned());
        let request_title = request_sidebar
            .as_ref()
            .map_or("PR", |sidebar| sidebar.read(cx).tab_title());
        let tabs = TabBar::new("git-navigation-tabs")
            .small()
            .underline()
            .flex_shrink_0()
            .selected_index(usize::from(requests_active))
            .child(
                Tab::new()
                    .label(conduit::i18n::text_static("files.commit"))
                    .debug_selector(|| "git-commit-tab".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.github_ui.sidebar_active = false;
                        cx.notify();
                    })),
            )
            .child(
                Tab::new()
                    .label(request_title)
                    .debug_selector(|| "context-pr".into())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.github_ui.sidebar_active = true;
                        cx.notify();
                    })),
            );
        if let Some(i) = self.selected_workspace {
            let workspace = &self.store.workspaces[i];
            pane = pane.child(
                sidebar::navigation_header()
                    .overflow_hidden()
                    .gap_1()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(tabs)
                    .child(div().flex_1())
                    .when(!requests_active, |header| {
                        header.child(
                            self.review_inbox_button(cx, None)
                                .debug_selector(|| "context-review".into()),
                        )
                    }),
            );
            let mut context = h_flex()
                .id("git-repository-context")
                .debug_selector(|| "git-repository-context".into())
                .h(px(34.))
                .border_b_1()
                .border_color(rgb(palette().border))
                .flex_shrink_0()
                .min_w_0()
                .items_center()
                .gap_1()
                .px_2()
                .child(div().flex_1().min_w_0().overflow_hidden().child(
                    self.navigation_workspace_picker(
                        "git-workspace",
                        workspace.name(),
                        workspace.path.clone(),
                        cx,
                    ),
                ));
            if requests_active {
                if let Some(sidebar) = request_sidebar {
                    let loading = sidebar.read(cx).is_loading();
                    context = context.child(
                        Button::new("request-refresh")
                            .debug_selector(|| "request-refresh".into())
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::RefreshCw)
                            .loading(loading)
                            .disabled(loading)
                            .tooltip(conduit::i18n::text_static("github.refresh"))
                            .accessibility_label(conduit::i18n::text_static("github.refresh"))
                            .on_click(move |_, _, cx| {
                                sidebar.update(cx, |sidebar, cx| sidebar.refresh(cx));
                            }),
                    );
                }
            } else {
                context = context.child(
                    Button::new("git-refresh")
                        .debug_selector(|| "git-refresh".into())
                        .ghost()
                        .small()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .loading(self.git.probe.is_some())
                        .tooltip(conduit::i18n::text_static("git_ui.refresh_git_status"))
                        .accessibility_label(conduit::i18n::text_static(
                            "git_ui.refresh_git_status",
                        ))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_git(cx))),
                );
                if let Some(snapshot) = &self.git.snapshot {
                    context = context.child(self.git_staging_actions(snapshot, app.clone()));
                }
            }
            pane = pane.child(context);
        } else {
            pane = pane.child(
                sidebar::navigation_header()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static(
                        "files.git_changes_no_workspace_selected",
                    )),
            );
        }
        if let Some(content) = request_content {
            return pane.child(content).into_any_element();
        }
        pane.child(
            v_flex()
                .flex_1()
                .min_h(px(72.))
                .min_w_0()
                .child(self.git_panel(app.clone())),
        )
        .child(
            div()
                .id("git-commit-scroll")
                .debug_selector(|| "git-commit-section".into())
                .h(relative(0.42))
                .min_h(px(244.))
                .flex_shrink_1()
                .min_w_0()
                .border_t_1()
                .border_color(rgb(palette().border))
                .child(self.commit_editor(app, cx)),
        )
        .into_any_element()
    }

    pub(super) fn open_diff_document(
        &mut self,
        workspace: PathBuf,
        file: git::ChangedFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.store.workspaces.iter().any(|w| w.path == workspace) {
            return;
        }
        if let Some(id) = self
            .files
            .diffs
            .iter()
            .find(|(_, d)| d.workspace == workspace && d.file.path == file.path)
            .map(|(id, _)| *id)
        {
            if let Some(d) = self.files.diffs.get_mut(&id) {
                d.file = file.clone();
                d.view.update(cx, |view, cx| {
                    view.file = file;
                    view.reload(window, cx);
                });
            }
            self.select_document(DocumentTab::Diff(id), window, cx);
            return;
        }
        self.files.next_id += 1;
        let id = self.files.next_id;
        let host = cx.weak_entity();
        let view = cx.new(|cx| {
            GitDiffDialog::new_document(workspace.clone(), file.clone(), window, cx)
                .with_review(host, cx)
        });
        self.files.diffs.insert(
            id,
            DiffDocument {
                workspace,
                file,
                view,
            },
        );
        self.open_tabs.push(DocumentTab::Diff(id));
        while self.files.diffs.len() > 20 {
            let Some(old) = self.open_tabs.iter().find_map(|t| match t {
                DocumentTab::Diff(old) if *old != id => Some(*old),
                _ => None,
            }) else {
                break;
            };
            self.files.diffs.remove(&old);
            self.open_tabs.retain(|t| *t != DocumentTab::Diff(old));
        }
        self.select_document(DocumentTab::Diff(id), window, cx);
    }

    /// Refresh only matching HEAD/worktree diffs; branch comparison dialogs keep their own lifecycle.
    pub(super) fn refresh_diff_documents(
        &mut self,
        path: &Path,
        snapshot: &git::Snapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_diff;
        for (id, d) in self
            .files
            .diffs
            .iter_mut()
            .filter(|(_, d)| d.workspace == path)
        {
            let file = snapshot
                .files
                .iter()
                .find(|f| f.path == d.file.path)
                .cloned()
                .unwrap_or_else(|| git::ChangedFile {
                    index: ' ',
                    worktree: ' ',
                    path: d.file.path.clone(),
                    original: None,
                });
            if d.file != file || active == Some(*id) {
                d.file = file.clone();
                d.view.update(cx, |view, cx| {
                    view.file = file;
                    view.reload(window, cx);
                });
            }
        }
    }

    pub(super) fn render_diff_document(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = self.active_diff else {
            return div().into_any_element();
        };
        let Some(document) = self.files.diffs.get(&id) else {
            return div()
                .child(conduit::i18n::text_static("files.diff_tab_closed"))
                .into_any_element();
        };
        v_flex()
            .id("git-diff-document")
            .debug_selector(|| "git-diff-document".into())
            .key_context("GitDiffDocument")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .on_action(cx.listener(move |this, _: &CloseDocument, window, cx| {
                this.close_documents(&[DocumentTab::Diff(id)], window, cx)
            }))
            .child(
                h_flex()
                    .h(px(30.))
                    .px_3()
                    .flex_shrink_0()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(ui_text_size(11.))
                            .child(document.workspace.display().to_string()),
                    )
                    .child(
                        Button::new("diff-return-chat")
                            .ghost()
                            .small()
                            .label(conduit::i18n::text_static("files.back_to_chat"))
                            .cursor_pointer()
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(id) = this.selected_session {
                                    this.select_document(DocumentTab::Chat(id), window, cx);
                                } else {
                                    this.active_diff = None;
                                    this.input.read(cx).focus_handle(cx).focus(window, cx);
                                    cx.notify();
                                }
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().min_w_0().child(
                    document
                        .view
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                ),
            )
            .when_some(self.document_attention(cx), |page, banner| {
                page.child(banner)
            })
            .into_any_element()
    }

    fn document_attention(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some((id, count)) = self
            .session_control
            .runs
            .iter()
            .filter(|(_, r)| !r.approvals.is_empty())
            .map(|(id, r)| (*id, r.approvals.len()))
            .min_by_key(|(id, _)| *id)
        {
            let title = self
                .session(id)
                .map(|s| single_line_title(&s.title))
                .unwrap_or_default();
            Some(
                Button::new("document-pending-approval")
                    .debug_selector(|| "file-pending-approval".into())
                    .label(conduit::i18n::format(
                        "files.has_pending_approvals_return_to_chat_to",
                        &[
                            ("title", title.to_string()),
                            ("count", format!("{}", count)),
                        ],
                    ))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_document(DocumentTab::Chat(id), window, cx)
                    }))
                    .into_any_element(),
            )
        } else {
            self.session_control
                .unread_completed
                .iter()
                .copied()
                .find(|id| {
                    self.session(*id).is_some_and(|s| {
                        s.messages.last().is_some_and(|m| m.kind == ItemKind::Error)
                    })
                })
                .map(|id| {
                    Button::new("document-task-error")
                        .label(conduit::i18n::text_static(
                            "files.background_task_failed_return_to_chat_for",
                        ))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_document(DocumentTab::Chat(id), window, cx)
                        }))
                        .into_any_element()
                })
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, DocumentTab, FileKey, Store, Workspace};
    use gpui_kit::{AppContext, Modifiers, MouseButton, TestAppContext, px};
    use std::{
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "conduit-file-tabs-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("alpha/src")).unwrap();
            std::fs::create_dir_all(root.join("beta/src")).unwrap();
            std::fs::write(
                root.join("alpha/src/main.rs"),
                "fn main() {\n    println!(\"中文\");\n}\n",
            )
            .unwrap();
            std::fs::write(root.join("alpha/README.md"), "# Project\n").unwrap();
            std::fs::write(root.join("beta/src/main.rs"), "// other workspace\n").unwrap();
            Self(root)
        }
        fn store(&self) -> Store {
            let a = super::super::tests::session();
            let mut b = a.clone();
            b.id = 2;
            Store {
                version: 2,
                workspaces: vec![
                    Workspace {
                        path: self.0.join("alpha"),
                        sessions: vec![a],
                    },
                    Workspace {
                        path: self.0.join("beta"),
                        sessions: vec![b],
                    },
                ],
                ..Default::default()
            }
        }
        fn key(&self, workspace: &str, file: &str) -> FileKey {
            FileKey {
                workspace: self.0.join(workspace),
                relative: file.into(),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let path = self.0.canonicalize().unwrap();
            assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            for attempt in 0..50 {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => return,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                    Err(e) if attempt == 49 => panic!("cleanup failed: {e}"),
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
                }
            }
        }
    }

    #[gpui_kit::test]
    fn chat_file_previews_cross_workspaces_without_notifications(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let outside = fixture.0.join("gamma/docs/notes 100%#1.md");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, "# External workspace\n").unwrap();
        let mut store = fixture.store();
        for workspace in &mut store.workspaces {
            workspace.path = workspace.path.canonicalize().unwrap();
        }
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1), DocumentTab::Chat(2)];
                app.input
                    .update(cx, |input, cx| input.set_value("draft alpha", window, cx));
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        let notices = cx.update(|_, cx| app.read(cx).notifications.center.records().len());
        let other = fixture.0.join("beta/src/main.rs");
        // Exercise the alias explicitly even when TEMP itself uses a long path.
        // Filesystems without 8.3 aliases return the original long path instead.
        #[cfg(windows)]
        let other = short_path(&other);
        let other_path = super::display_path(&other);
        let other_url = format!(
            "file:///{}#L1",
            other_path.replace('\\', "/").trim_start_matches('/')
        );
        let outside_link = "../gamma/docs/notes%20100%25%231.md#L1";
        for (link, attachment, registered) in [
            ("../beta/src/main.rs", false, true),
            (other_url.as_str(), false, true),
            (other_path.as_str(), true, true),
            (outside_link, false, false),
            (outside.to_str().unwrap(), true, false),
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.select_document(DocumentTab::Chat(1), window, cx);
                    if attachment {
                        app.open_chat_file(1, link, window, cx);
                    } else {
                        app.open_chat_link(1, link, window, cx);
                    }
                    assert!(app.active_file.is_some(), "{link}");
                    assert_eq!(
                        app.selected_workspace,
                        Some(usize::from(registered)),
                        "{link}"
                    );
                    assert_eq!(app.store.workspaces.len(), 2);
                    assert!(app.message.is_none());
                    assert_eq!(app.notifications.center.records().len(), notices);
                });
            });
            cx.run_until_parked();
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let id = app.active_file.unwrap();
                    let document = &app.files.documents[&id];
                    assert!(!document.loading);
                    assert!(document.error.is_none(), "{:?}", document.error);
                    assert!(document.text.contains(if registered {
                        "other workspace"
                    } else {
                        "External workspace"
                    }));
                    if registered {
                        // Explorer and transcript links must reuse the same tab.
                        let key = FileKey {
                            workspace: app.store.workspaces[1].path.clone(),
                            relative: "src/main.rs".into(),
                        };
                        app.open_workspace_file(key, true, window, cx);
                        assert_eq!(app.active_file, Some(id));
                    } else {
                        app.select_document(DocumentTab::Chat(1), window, cx);
                        app.select_document(DocumentTab::File(id), window, cx);
                        assert_eq!(app.active_file, Some(id));
                    }
                    app.close_documents(&[DocumentTab::File(id)], window, cx);
                    assert_eq!(app.selected_session, Some(if registered { 2 } else { 1 }));
                    if !registered {
                        assert_eq!(app.input.read(cx).value(), "draft alpha");
                    }
                    assert!(app.message.is_none());
                    assert_eq!(app.notifications.center.records().len(), notices);
                });
            });
        }
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    // Protect citation navigation across asynchronous loading/parsing and tab reuse.
    #[gpui_kit::test]
    fn chat_citations_reveal_loaded_source_and_markdown(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let code: String = (1..=220).map(|n| format!("// line {n}: 中文\n")).collect();
        let prose: String = (1..=220)
            .map(|n| format!("Paragraph {n}: **中文**.\n\n"))
            .collect();
        for (path, text) in [
            ("src/main.rs", code.clone()),
            ("README.md", prose),
            ("code.md", format!("# Code\n\n```rust\n{code}```\n")),
            ("stock-code.md", format!("# Code\n\n```rust\n{code}```\n")),
        ] {
            std::fs::write(fixture.0.join("alpha").join(path), text).unwrap();
        }
        let mut store = fixture.store();
        store.ui.code_line_numbers = true;
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1)];
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        for (path, citation, numbered) in [
            ("src/main.rs", ":180:4", true),
            ("README.md", "#L359", true),
            ("code.md", "#L183C4", true),
            ("stock-code.md", "#L183C4", false),
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.store.ui.code_line_numbers = numbered;
                    app.open_chat_link(1, &format!("{path}{citation}"), window, cx);
                    assert!(app.files.documents[&app.active_file.unwrap()].loading);
                });
            });
            // File IO, Markdown parsing, layout and deferred reveals finish on
            // separate turns. Drain each stage before observing the viewport.
            for _ in 0..4 {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    let _ = window.draw(cx);
                });
            }
            let id = cx.update(|_, cx| {
                let app = app.read(cx);
                let id = app.active_file.unwrap();
                let document = &app.files.documents[&id];
                assert!(!document.loading);
                assert!(document.error.is_none(), "{:?}", document.error);
                let scroll = &document.preview.read(cx).scroll;
                assert!(
                    scroll.offset().y < -scroll.bounds().size.height,
                    "{path}: {:?}",
                    scroll.offset()
                );
                id
            });
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.open_chat_link(1, &format!("{path}#L1"), window, cx);
                    assert_eq!(app.active_file, Some(id));
                    assert_eq!(app.files.documents.len(), 1);
                });
            });
            for _ in 0..3 {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    let _ = window.draw(cx);
                });
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let scroll = &app.files.documents[&id].preview.read(cx).scroll;
                    assert_eq!(scroll.offset().y, px(0.), "{path}");
                    app.open_chat_link(1, &format!("{path}#L999999C999999"), window, cx);
                });
            });
            for _ in 0..3 {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    let _ = window.draw(cx);
                });
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    let scroll = &app.files.documents[&id].preview.read(cx).scroll;
                    assert!(
                        scroll.offset().y < -scroll.bounds().size.height,
                        "{path}: EOF"
                    );
                    app.close_documents(&[DocumentTab::File(id)], window, cx);
                });
            });
        }
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    #[cfg(windows)]
    fn short_path(path: &Path) -> PathBuf {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

        let path: Vec<u16> = path
            .canonicalize()
            .unwrap()
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // Query the required capacity, including the terminating null.
        let capacity = unsafe { GetShortPathNameW(path.as_ptr(), std::ptr::null_mut(), 0) };
        assert_ne!(capacity, 0, "{}", std::io::Error::last_os_error());
        let mut result = vec![0; capacity as usize];
        let length = unsafe { GetShortPathNameW(path.as_ptr(), result.as_mut_ptr(), capacity) };
        assert!(
            length > 0 && length < capacity,
            "{}",
            std::io::Error::last_os_error()
        );
        PathBuf::from(std::ffi::OsString::from_wide(&result[..length as usize]))
    }

    #[gpui_kit::test]
    fn file_navigation_preview_pin_close_and_workspace_context(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let store = fixture.store();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1)];
                app.input
                    .update(cx, |input, cx| input.set_value("draft alpha", window, cx));
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let sidebar = cx.debug_bounds("sidebar").unwrap();
        let workspace_row = cx.debug_bounds("workspace-item-0").unwrap();
        assert_eq!(workspace_row.left() - sidebar.left(), px(8.));
        let point = cx.debug_bounds("nav-files").unwrap().center();
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let explorer = cx.debug_bounds("file-explorer").unwrap();
        let file_row = cx.debug_bounds("file-tree-entry-src").unwrap();
        assert_eq!(
            file_row.left() - explorer.left(),
            workspace_row.left() - sidebar.left()
        );
        let point = file_row.center();
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let selector = format!(
            "file-tree-entry-{}",
            Path::new("src").join("main.rs").display()
        );
        let point = cx
            .debug_bounds(Box::leak(selector.into_boxed_str()))
            .unwrap()
            .center();
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("file-document").is_some());
        cx.update(|_, cx| {
            let a = app.read(cx);
            let id = a.active_file.unwrap();
            assert_eq!(a.files.preview, Some(id));
            assert!(!a.files.documents[&id].highlights.is_empty());
            assert!(a.files.documents[&id].text.contains("中文"));
            assert_eq!(a.selected_session, Some(1));
            assert_eq!(a.input.read(cx).value(), "draft alpha");
        });
        let source = cx.debug_bounds("file-source-view").unwrap();
        let start = source.origin + gpui_kit::point(px(39.), px(12.));
        let end = source.origin + gpui_kit::point(px(190.), px(35.));
        cx.simulate_mouse_move(start, None, Modifiers::default());
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        let selected = cx.update(gpui_kit::base::TextSelection::selected_text);
        assert!(
            selected.contains('\n') && selected.contains("println"),
            "selection: {selected:?}"
        );
        cx.update(|window, cx| {
            cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("SENTINEL".into()));
            window.dispatch_action(Box::new(gpui_kit::component::input::Copy), cx);
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| cx
                .read_from_clipboard()
                .and_then(|c| c.text())
                .unwrap_or_default()),
            selected
        );
        // Preview replacement preserves the slot; pinned documents are not replaced.
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.open_workspace_file(fixture.key("alpha", "README.md"), false, window, cx);
                let id = a.active_file.unwrap();
                assert_eq!(a.files.documents.len(), 1);
                a.open_workspace_file(fixture.key("alpha", "README.md"), true, window, cx);
                assert_eq!(a.active_file, Some(id));
                assert_eq!(a.files.preview, None);
                a.open_workspace_file(fixture.key("alpha", "src/main.rs"), false, window, cx);
                assert_eq!(a.files.documents.len(), 2);
                a.close_documents(&[DocumentTab::File(id)], window, cx);
                assert!(a.active_file.is_some());
                assert_eq!(a.files.documents.len(), 1);
                a.close_documents(&[DocumentTab::File(a.active_file.unwrap())], window, cx);
                assert_eq!(a.active_file, None);
                assert_eq!(a.selected_session, Some(1));
                assert_eq!(a.input.read(cx).value(), "draft alpha");
                // File context may change workspace, but it must not open/evict chats or consume unread replies.
                a.store.ui.max_tabs = 1;
                a.session_control.unread_completed.insert(2);
                a.open_workspace_file(fixture.key("beta", "src/main.rs"), true, window, cx);
                assert_eq!(a.selected_workspace, Some(1));
                assert_eq!(a.selected_session, Some(2));
                assert!(a.session_control.unread_completed.contains(&2));
                assert!(a.open_tabs.contains(&DocumentTab::Chat(1)));
                assert!(!a.open_tabs.contains(&DocumentTab::Chat(2)));
                assert_eq!(a.drafts.get(&1).unwrap(), "draft alpha");
                a.select_document(DocumentTab::Chat(1), window, cx);
                assert_eq!(a.input.read(cx).value(), "draft alpha");
                let tab = a
                    .open_tabs
                    .iter()
                    .copied()
                    .find(|t| matches!(t, DocumentTab::File(_)))
                    .unwrap();
                a.select_document(tab, window, cx);
                assert_eq!(a.selected_workspace, Some(1));
                // Closing the beta preview must restore beta, even though alpha
                // is the only open chat (and the global tab limit is one).
                a.input
                    .update(cx, |input, cx| input.set_value("draft beta", window, cx));
                a.close_documents(&[tab], window, cx);
                assert_eq!(a.active_document(), Some(DocumentTab::Chat(2)));
                assert_eq!(a.selected_workspace, Some(1));
                assert_eq!(a.input.read(cx).value(), "draft beta");
                assert_eq!(a.drafts.get(&1).unwrap(), "draft alpha");
                a.open_workspace_file(fixture.key("beta", "src/main.rs"), true, window, cx);
                a.forget_workspace_files(&fixture.0.join("beta"));
                assert_eq!(a.active_file, None);
                assert!(fixture.0.join("beta/src/main.rs").exists());
                a.open_workspace_file(fixture.key("beta", "src/main.rs"), false, window, cx);
                let all = a.open_tabs.clone();
                a.close_documents(&all, window, cx);
                assert!(a.open_tabs.is_empty());
                assert_eq!(a.selected_workspace, Some(1));
                assert_eq!(a.selected_session, None);
                // An empty workspace also stays selected after its file closes.
                a.store.workspaces[1].sessions.clear();
                a.open_workspace_file(fixture.key("beta", "src/main.rs"), false, window, cx);
                a.push_tab(1);
                let file = a.active_document().unwrap();
                a.close_documents(&[file], window, cx);
                assert_eq!(a.selected_workspace, Some(1));
                assert_eq!(a.selected_session, None);
            })
        });
        cx.run_until_parked();
        // Collapsing keeps the activity rail but releases sidebar space.
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.plugins.set_collapsed(true);
                cx.notify();
            })
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("file-explorer").is_none());
        assert!(cx.debug_bounds("nav-files").is_some());
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod git_navigation_tests {
    use super::{ConduitApp, DocumentTab, Store, Workspace};
    use crate::app::{NoticeLevel, NoticeSource};
    use crate::git;
    use gpui_kit::test::TestQueryExt as _;
    use gpui_kit::{
        AppContext, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext,
        component::WindowExt as _,
    };
    use std::{
        path::PathBuf,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "conduit-git-navigation-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(root.join("src")).unwrap();
            let repo = Self(root);
            repo.git(&["init", "-q"]);
            repo.git(&["config", "user.name", "Conduit Test"]);
            repo.git(&["config", "user.email", "test@example.invalid"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            std::fs::write(
                repo.0.join("src/main.rs"),
                "fn main() {\n    println!(\"before\");\n}\n",
            )
            .unwrap();
            repo.git(&["add", "--", "src/main.rs"]);
            repo.git(&["commit", "-qm", "initial"]);
            std::fs::write(
                repo.0.join("src/main.rs"),
                "fn main() {\n    println!(\"after\");\n}\n",
            )
            .unwrap();
            std::fs::write(repo.0.join("new file.txt"), "new content\n").unwrap();
            repo
        }
        fn git(&self, args: &[&str]) {
            let output = std::process::Command::new(git::resolve_git().unwrap())
                .current_dir(&self.0)
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fn store(&self) -> Store {
            let one = super::super::tests::session();
            let mut two = one.clone();
            two.id = 2;
            Store {
                version: 2,
                workspaces: vec![
                    Workspace {
                        path: self.0.clone(),
                        sessions: vec![one],
                    },
                    Workspace {
                        path: self.0.join("src"),
                        sessions: vec![two],
                    },
                ],
                ..Default::default()
            }
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let path = self.0.canonicalize().unwrap();
            assert!(path.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            for attempt in 0..100 {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => return,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                    Err(e) if attempt == 99 => panic!("cleanup failed: {e}"),
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        }
    }
    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
    }
    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let point = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        draw(cx);
    }
    fn wait_git(cx: &mut VisualTestContext, app: &Entity<ConduitApp>) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let done = cx.update(|window, cx| {
                app.update(cx, |a, cx| {
                    a.poll_git_actions(window, cx);
                    a.poll_branches(cx);
                    a.poll_git(window, cx);
                    a.git.probe.is_none()
                        && a.git.operation.is_none()
                        && a.git.branch_probe.is_none()
                })
            });
            cx.run_until_parked();
            if done {
                break;
            }
            assert!(Instant::now() < deadline, "Git probe timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        draw(cx);
    }

    #[gpui_kit::test]
    fn file_tree_drag_git_status_tag_cache_and_discard(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        use super::TreeGitStatus;
        use gpui_kit::{point, px};
        cx.update(gpui_kit::init);
        let repo = Repo::new();
        repo.git(&["tag", "v1.0.0"]);
        std::fs::write(repo.0.join("added.txt"), "added\n").unwrap();
        repo.git(&["add", "added.txt"]);
        let store = repo.store();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = true;
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1)];
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        draw(cx);
        click(cx, "nav-files");
        wait_git(cx, &app);
        cx.update(|_, cx| {
            let a = app.read(cx);
            let statuses = &a.files.git_status;
            assert!(statuses[&repo.0.join("src/main.rs")] == TreeGitStatus::Modified);
            assert!(statuses[&repo.0.join("src")] == TreeGitStatus::Modified);
            assert!(statuses[&repo.0.join("added.txt")] == TreeGitStatus::Added);
            assert!(statuses[&repo.0.join("new file.txt")] == TreeGitStatus::Untracked);
            assert_eq!(a.git.local_tags[&repo.0], vec!["v1.0.0"]);
        });
        let source = cx
            .debug_bounds("file-tree-entry-new file.txt")
            .unwrap()
            .center();
        let target = cx.debug_bounds("composer").unwrap().center();
        cx.simulate_mouse_down(source, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(
            point(source.x + px(12.), source.y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        draw(cx);
        cx.simulate_mouse_move(target, Some(MouseButton::Left), Modifiers::default());
        draw(cx);
        cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert_eq!(a.current_files().unwrap().references.len(), 1);
            assert!(
                a.current_files().unwrap().references[0]
                    .path
                    .ends_with("new file.txt")
            );
            assert!(a.input.read(cx).value().is_empty());
            assert_eq!(a.selected_session, Some(1));
        });
        // Status refreshes and navigation reuse tags until explicit refresh.
        repo.git(&["tag", "v1.0.1"]);
        click(cx, "nav-git");
        wait_git(cx, &app);
        // Re-selecting the repository's chat does not reorder the global tabs.
        // Closing its diff must return to that chat, not the later foreign tab.
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.switch(1, Some(2), window, cx);
                a.switch(0, Some(1), window, cx);
                assert_eq!(a.open_tabs.last(), Some(&DocumentTab::Chat(2)));
                a.input.update(cx, |input, cx| {
                    input.set_value("repository draft", window, cx)
                });
                a.open_git_diff(
                    git::ChangedFile {
                        path: "src/main.rs".into(),
                        original: None,
                        index: ' ',
                        worktree: 'M',
                    },
                    window,
                    cx,
                );
                assert_eq!(a.selected_workspace, Some(0));
                assert_eq!(a.selected_session, Some(1));
            });
        });
        wait_git(cx, &app);
        let diff = cx.update(|_, cx| app.read(cx).active_diff.unwrap());
        let close_selector: &'static str =
            Box::leak(format!("diff-tab-close-{diff}").into_boxed_str());
        let close = cx.debug_bounds(close_selector).unwrap().center();
        cx.simulate_mouse_move(close, None, Modifiers::default());
        cx.simulate_mouse_down(close, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(close, MouseButton::Left, Modifiers::default());
        draw(cx);
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                assert_eq!(a.active_document(), Some(DocumentTab::Chat(1)));
                assert_eq!(a.selected_workspace, Some(0));
                assert_eq!(a.git.workspace.as_ref(), Some(&repo.0));
                assert_eq!(a.input.read(cx).value(), "repository draft");
                a.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            });
        });
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                assert_eq!(a.git.local_tags[&repo.0], vec!["v1.0.0"]);
                a.git.drafts[&repo.0]
                    .tag
                    .update(cx, |tag, cx| tag.set_value("v2.0.0", window, cx));
                cx.notify();
            });
        });
        draw(cx);
        click(cx, "git-tag-list");
        assert!(cx.update(|window, _| {
            !window
                .find_all_by_label("v1.0.0  ·  本地 · 未查询远程")
                .is_empty()
        }));
        cx.simulate_keystrokes("escape");
        draw(cx);
        click(cx, "git-tag-refresh-remote");
        wait_git(cx, &app);
        cx.update(|_, cx| {
            assert_eq!(
                app.read(cx).git.local_tags[&repo.0],
                vec!["v1.0.1", "v1.0.0"]
            )
        });
        // The actual context menu must lead to a confirmation before any edit.
        let row = cx
            .debug_bounds("git-change-file-src/main.rs")
            .unwrap()
            .center();
        cx.simulate_mouse_move(row, None, Modifiers::default());
        cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::default());
        draw(cx);
        let rollback =
            cx.update(|window, _| window.find_by_label("回滚文件修改…").bounds().center());
        cx.simulate_mouse_down(rollback, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(rollback, MouseButton::Left, Modifiers::default());
        draw(cx);
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        assert!(cx.update(|_, cx| app.read(cx).git.operation.is_none()));
        assert!(
            std::fs::read_to_string(repo.0.join("src/main.rs"))
                .unwrap()
                .contains("after")
        );
        cx.simulate_keystrokes("enter");
        wait_git(cx, &app);
        assert!(
            std::fs::read_to_string(repo.0.join("src/main.rs"))
                .unwrap()
                .contains("before")
        );
        assert!(repo.0.join("new file.txt").exists());
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert_eq!(a.git.snapshot.as_ref().unwrap().staged_count(), 1);
            assert!(!a.files.git_status.contains_key(&repo.0.join("src/main.rs")));
        });
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn git_stages_whole_files_after_idle_and_requires_manual_additions(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let repo = Repo::new();
        std::fs::write(repo.0.join("added.txt"), "manually staged\n").unwrap();
        repo.git(&["add", "--", "src/main.rs", "added.txt"]);
        std::fs::write(repo.0.join("src/main.rs"), "latest tracked edit\n").unwrap();
        std::fs::write(repo.0.join("added.txt"), "latest manually added edit\n").unwrap();
        let store = repo.store();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1)];
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        draw(cx);
        click(cx, "nav-git");
        wait_git(cx, &app);
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                let snapshot = a.git.snapshot.as_ref().unwrap();
                assert_eq!(snapshot.staged_count(), 0);
                assert_eq!(snapshot.unstaged_count(), 2);
                assert_eq!(snapshot.untracked_count(), 1);
                assert!(a.git.auto_stage_after_refresh);
                // Resume the pending request without requesting another snapshot.
                a.writable = true;
                a.poll_git(window, cx);
            });
        });
        wait_git(cx, &app);
        cx.update(|_, cx| {
            let a = app.read(cx);
            let snapshot = a.git.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.staged_count(), 2);
            assert_eq!(snapshot.unstaged_count(), 0);
            assert_eq!(snapshot.untracked_count(), 1);
            assert!(snapshot.staged_diff.contains("+latest tracked edit"));
            assert!(snapshot.staged_diff.contains("+latest manually added edit"));
            assert!(!a.git.auto_stage_after_refresh);
        });
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.unstage_git_files(vec!["added.txt".into()], cx)
            });
        });
        wait_git(cx, &app);
        cx.update(|_, cx| app.update(cx, |a, cx| a.refresh_git(cx)));
        wait_git(cx, &app);
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                let snapshot = a.git.snapshot.as_ref().unwrap();
                assert_eq!(snapshot.staged_count(), 1);
                assert_eq!(snapshot.untracked_count(), 2);
                assert!(
                    snapshot
                        .files
                        .iter()
                        .any(|f| f.path == "added.txt" && f.untracked())
                );
                a.stage_git_files(vec!["new file.txt".into()], cx);
            });
        });
        wait_git(cx, &app);
        std::fs::write(repo.0.join("new file.txt"), "edited after manual staging\n").unwrap();
        cx.update(|_, cx| app.update(cx, |a, cx| a.refresh_git(cx)));
        wait_git(cx, &app);
        cx.update(|_, cx| {
            let a = app.read(cx);
            let snapshot = a.git.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.staged_count(), 2);
            assert_eq!(snapshot.unstaged_count(), 0);
            assert_eq!(snapshot.untracked_count(), 1);
            assert!(
                snapshot
                    .staged_diff
                    .contains("+edited after manual staging")
            );
            assert!(
                snapshot
                    .files
                    .iter()
                    .any(|f| f.path == "added.txt" && f.untracked())
            );
        });
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn commit_button_requires_checks_and_commits_only_checked_staged_files(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let repo = Repo::new();
        repo.git(&["add", "."]);
        let store = repo.store();
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = true;
                app.startup = None;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.open_tabs = vec![DocumentTab::Chat(1)];
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        draw(cx);
        click(cx, "nav-git");
        wait_git(cx, &app);
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.git.drafts[&repo.0]
                    .input
                    .update(cx, |input, cx| input.set_value("only main", window, cx));
                cx.notify();
            })
        });
        draw(cx);
        click(cx, "git-commit");
        assert!(cx.update(|_, cx| app.read(cx).git.operation.is_none()));
        assert!(cx.debug_bounds("git-stage-all").is_none());
        click(cx, "git-select-src/main.rs");
        click(cx, "git-commit");
        wait_git(cx, &app);
        cx.update(|_, cx| {
            let a = app.read(cx);
            let snapshot = a.git.snapshot.as_ref().unwrap();
            assert_eq!(snapshot.staged_count(), 1);
            assert_eq!(snapshot.files[0].path, "new file.txt");
            let notice = a
                .notifications
                .center
                .records()
                .iter()
                .find(|notice| notice.source == NoticeSource::Git)
                .unwrap();
            assert_ne!(notice.level, NoticeLevel::Error, "{}", notice.message);
            assert!(a.git.drafts[&repo.0].input.read(cx).value().is_empty());
        });
    }
}

impl FileBrowser {
    pub(super) fn refresh_language(&mut self, window: &mut Window, cx: &mut App) {
        for document in self.documents.values() {
            document
                .preview
                .update(cx, |preview, cx| preview.refresh_language(window, cx));
        }
        for document in self.diffs.values() {
            document.view.update(cx, |_, cx| cx.notify());
        }
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use core::prelude::v1::test;

    fn entry(path: &str, directory: bool) -> Entry {
        let relative = PathBuf::from(path);
        Entry {
            name: relative.file_name().unwrap().to_string_lossy().into_owned(),
            relative,
            directory,
            link: false,
        }
    }
    fn directory(entries: Vec<Entry>) -> Directory {
        Directory {
            entries,
            stamp: Some(Stamp {
                modified: None,
                len: 0,
            }),
            ..Default::default()
        }
    }
    fn fixture() -> Explorer {
        Explorer {
            directories: [
                (PathBuf::new(), directory(vec![entry("src", true)])),
                (
                    PathBuf::from("src"),
                    directory(vec![entry("src/com", true)]),
                ),
                (
                    PathBuf::from("src/com"),
                    directory(vec![entry("src/com/example", true)]),
                ),
                (
                    PathBuf::from("src/com/example"),
                    directory(vec![entry("src/com/example/main.rs", false)]),
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }
    fn entries(rows: &[TreeRow]) -> Vec<(String, usize)> {
        rows.iter()
            .filter_map(|row| match row {
                TreeRow::Entry(entry, depth, _) => Some((entry.name.clone(), *depth)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn compact_paths_toggle_without_losing_expansion_or_real_file_paths() {
        let mut tree = fixture();
        assert_eq!(
            entries(&tree.rows(true)),
            vec![("src/com/example".into(), 0)]
        );
        assert!(tree.visible_directories.contains(Path::new("src/com")));
        assert!(tree.toggle(Path::new("src"), Path::new("src/com/example")));
        let rows = tree.rows(true);
        assert_eq!(
            entries(&rows),
            vec![("src/com/example".into(), 0), ("main.rs".into(), 1)]
        );
        assert!(
            matches!(&rows[1], TreeRow::Entry(e, _, _) if e.relative == Path::new("src/com/example/main.rs"))
        );
        assert_eq!(
            entries(&tree.rows(false)),
            vec![
                ("src".into(), 0),
                ("com".into(), 1),
                ("example".into(), 2),
                ("main.rs".into(), 3)
            ]
        );
        assert!(!tree.toggle(Path::new("src"), Path::new("src/com/example")));
        assert_eq!(entries(&tree.rows(true)).len(), 1);
    }

    #[test]
    fn compact_paths_stop_at_branches_files_links_errors_and_unknown_children() {
        let mut tree = fixture();
        tree.directories
            .get_mut(Path::new("src/com"))
            .unwrap()
            .entries
            .push(entry("src/com/other", true));
        assert_eq!(entries(&tree.rows(true))[0].0, "src/com");
        tree.directories
            .get_mut(Path::new("src"))
            .unwrap()
            .entries
            .push(entry("src/config.json", false));
        tree.cached_rows = None;
        assert_eq!(entries(&tree.rows(true))[0].0, "src");
        let mut tree = fixture();
        tree.directories.get_mut(Path::new("src")).unwrap().entries[0].link = true;
        assert_eq!(entries(&tree.rows(true))[0].0, "src");
        let mut tree = fixture();
        tree.directories.get_mut(Path::new("src")).unwrap().error = Some("unreadable".into());
        assert_eq!(entries(&tree.rows(true))[0].0, "src");
        let mut tree = fixture();
        tree.directories.remove(Path::new("src/com"));
        assert_eq!(entries(&tree.rows(true))[0].0, "src/com");
        assert!(tree.visible_directories.contains(Path::new("src/com")));
    }

    #[test]
    fn loaded_middle_segments_stay_open_when_compaction_is_disabled() {
        let mut tree = fixture();
        tree.expanded.insert(PathBuf::from("src"));
        assert_eq!(entries(&tree.rows(true)).last().unwrap().0, "main.rs");
        assert_eq!(
            entries(&tree.rows(false)).last().unwrap(),
            &("main.rs".into(), 3)
        );
    }

    #[test]
    fn grouping_preserves_first_opened_group_and_within_group_order() {
        use DocumentTab::*;
        let opened = vec![Chat(2), File(1), Settings, Chat(1), Diff(2), File(2)];
        let grouped = grouped_tabs(&opened, |tab| match tab {
            Settings => None,
            Chat(id) | File(id) | Diff(id) | Usage(id) | PullRequest(id) => {
                Some(PathBuf::from(format!("workspace-{id}")))
            }
        });
        assert_eq!(
            grouped,
            vec![Chat(2), Diff(2), File(2), File(1), Chat(1), Settings]
        );
        assert_eq!(
            opened,
            vec![Chat(2), File(1), Settings, Chat(1), Diff(2), File(2)]
        );
        assert_eq!(grouped.iter().position(|t| *t == File(1)), Some(3));
    }
}
