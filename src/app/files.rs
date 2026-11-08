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
}
struct Document {
    key: FileKey,
    text: SharedString,
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
struct DiffDocument {
    workspace: PathBuf,
    file: git::ChangedFile,
    view: Entity<GitDiffDialog>,
}
#[derive(Default)]
pub(super) struct FileBrowser {
    tab_scroll: ScrollHandle,
    tab_layout: std::cell::RefCell<(Vec<DocumentTab>, Option<DocumentTab>, Pixels)>,
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
    Entry(Entry, usize),
    Status(PathBuf, usize, String),
}

/// Streaming chat invalidations must not reshape a whole source document.
#[derive(Eq, PartialEq)]
struct FileViewSnapshot {
    document: Option<(u64, u64, u64, bool, Option<String>, bool)>,
    session: Option<u64>,
    approvals: Vec<(u64, usize)>,
    errors: Vec<u64>,
    appearance: (u32, u32, u32, Option<String>, u32),
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
                self.active_diff
                    .map(DocumentTab::Diff)
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
            DocumentTab::Settings => self.open_settings(window, cx),
            DocumentTab::Chat(id) => {
                self.active_diff = None;
                if let Some(index) = self
                    .store
                    .workspaces
                    .iter()
                    .position(|w| w.sessions.iter().any(|s| s.id == id))
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
                let Some(index) = self
                    .store
                    .workspaces
                    .iter()
                    .position(|w| w.path == key.workspace)
                else {
                    return;
                };
                // Switch context only when necessary, preserving drafts and chat scroll on same-workspace browsing.
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
                                .any(|s| s.id == *id && !s.archived)
                        })
                        .or_else(|| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .find(|s| !s.archived)
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
                                .any(|s| s.id == *id && !s.archived)
                        })
                        .or_else(|| {
                            self.store.workspaces[index]
                                .sessions
                                .iter()
                                .find(|s| !s.archived)
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
        let closing_active = self
            .active_document()
            .is_some_and(|tab| tabs.contains(&tab));
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
            if let DocumentTab::Diff(id) = tab {
                self.files.diffs.remove(id);
            }
            if let DocumentTab::File(id) = tab {
                self.files.documents.remove(id);
                if self.files.preview == Some(*id) {
                    self.files.preview = None;
                }
            }
        }
        if closing_active && !closing_settings {
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
            self.files.documents.remove(&old);
        }
        self.files.next_id += 1;
        let id = self.files.next_id;
        let scroll = gpui_kit::ScrollHandle::new();
        let markdown = is_markdown(&key.relative);
        let preview = cx.new(|cx| FilePreview::new(markdown, scroll.clone(), window, cx));
        self.files.documents.insert(
            id,
            Document {
                key,
                text: "".into(),
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
            self.files.documents.remove(&old);
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
        document.loading = true;
        document.sequence += 1;
        let sequence = document.sequence;
        let key = document.key.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    workspace_files::read(&key).map(|file| {
                        let names =
                            super::code_highlight::highlight_file(&key.relative, &file.text);
                        (file, names)
                    })
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
                    Ok((file, highlights)) => {
                        if document.text.as_ref() != file.text {
                            document.revision += 1;
                        }
                        document.text = file.text.into();
                        document.highlights = Arc::new(highlights);
                        document.stamp = Some(file.stamp);
                        document.encoding = file.encoding;
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
                    .chain(e.expanded.iter().cloned())
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
                        (p.as_os_str().is_empty() || e.expanded.contains(*p))
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
        fn flatten(explorer: &Explorer, path: &Path, depth: usize, rows: &mut Vec<TreeRow>) {
            if depth > 64 {
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
                rows.push(TreeRow::Entry(entry.clone(), depth));
                if entry.directory && explorer.expanded.contains(&entry.relative) {
                    flatten(explorer, &entry.relative, depth + 1, rows);
                }
            }
        }
        let rows = if let Some(rows) = &explorer.cached_rows {
            rows.clone()
        } else {
            let mut rows = Vec::new();
            flatten(explorer, Path::new(""), 0, &mut rows);
            let rows = Arc::new(rows);
            explorer.cached_rows = Some(rows.clone());
            rows
        };
        let app = cx.entity();
        let header = self.navigation_workspace_picker("files-workspace", name, root.clone(), cx);
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
                            TreeRow::Entry(entry, depth) => {
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
                                    .is_some_and(|e| e.expanded.contains(&entry.relative));
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
                                            {
                                                e.cached_rows = None;
                                                if !e.expanded.remove(&click_key.relative) {
                                                    e.expanded.insert(click_key.relative.clone());
                                                }
                                                if e.expanded.contains(&click_key.relative) {
                                                    this.load_directory(
                                                        click_key.workspace.clone(),
                                                        click_key.relative.clone(),
                                                        cx,
                                                    );
                                                }
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
            .child(
                sidebar::navigation_content().child(
                    Button::new("files-show-ignored")
                        .ghost()
                        .small()
                        .label(if show_ignored {
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
                                    d.sequence += 1;
                                }
                            }
                            this.refresh_tree(ignore_root.clone(), cx);
                        })),
                ),
            )
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
        let parent_label = parts.join(" / ");
        let text = document.text.clone();
        let copy = text.clone();
        let wrap = document.wrap;
        let status = if document.loading {
            conduit::i18n::text_static("files.reading").to_string()
        } else if document.error.is_some() {
            conduit::i18n::text_static("files.read_only_preview_unavailable").to_string()
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
                            .disabled(markdown)
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
                            .disabled(document.error.is_some() || document.loading)
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
            page = page.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .child(conduit::i18n::text_static("files.empty_file")),
            );
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

    pub(super) fn document_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self
            .active_document()
            .and_then(|t| self.open_tabs.iter().position(|item| *item == t));
        let active = self.active_document();
        {
            let mut layout = self.files.tab_layout.borrow_mut();
            if layout.0 != self.open_tabs || layout.1 != active {
                layout.0 = self.open_tabs.clone();
                layout.1 = active;
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
            .when_some(selected, |bar, i| bar.selected_index(i));
        for tab in self.open_tabs.iter().copied() {
            let (title, tooltip, icon, running, unread, selector) = match tab {
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
                DocumentTab::Chat(id) => format!("session-tab-close-{id}"),
                DocumentTab::File(id) => format!("file-tab-close-{id}"),
                DocumentTab::Diff(id) => format!("diff-tab-close-{id}"),
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
        if self.open_tabs.is_empty() {
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
                                    this.open_tabs.iter().position(|item| *item == tab)
                                }) {
                                    this.files.tab_scroll.scroll_to_item(index);
                                    cx.notify();
                                }
                            });
                        });
                    }
                });
            })
            .child(bar.suffix(self.detail_toggle(cx)))
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
                                if let Some(i) = this.open_tabs.iter().position(|t| *t == target) {
                                    let tabs = this.open_tabs[i + 1..].to_vec();
                                    this.close_documents(&tabs, window, cx);
                                }
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(conduit::i18n::text_static("files.close_tabs_to_the_left"))
                        .on_click(move |_, window, cx| {
                            left.update(cx, |this, cx| {
                                if let Some(i) = this.open_tabs.iter().position(|t| *t == target) {
                                    let tabs = this.open_tabs[..i].to_vec();
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
        if self.branch_navigation_open() {
            let collapsed = self.plugins.collapsed();
            let _ = self.select_navigation(&ViewId::new(builtins::WORKSPACES).unwrap());
            self.plugins.set_collapsed(collapsed);
        }
    }

    pub(super) fn branch_navigation_open(&self) -> bool {
        self.plugins.is_selected(builtins::BRANCHES)
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
        self.open_navigation(&ViewId::new(builtins::BRANCHES).unwrap(), cx);
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
            .ghost()
            .small()
            .label(name)
            .icon(IconName::ChevronDown)
            .tooltip(root.display().to_string())
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
                                            this.store.workspaces[i]
                                                .sessions
                                                .iter()
                                                .any(|s| s.id == *id && !s.archived)
                                        })
                                        .or_else(|| {
                                            this.store.workspaces[i]
                                                .sessions
                                                .iter()
                                                .find(|s| !s.archived)
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
        if let Some(i) = self.selected_workspace {
            let workspace = &self.store.workspaces[i];
            pane = pane.child(
                sidebar::navigation_header()
                    .overflow_hidden()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .h(px(26.))
                            .flex_shrink_0()
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_strong))
                            .text_color(rgb(palette().activity_running))
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .child(conduit::i18n::text_static("files.commit")),
                    )
                    .child(div().flex_1())
                    .child(self.navigation_workspace_picker(
                        "git-workspace",
                        workspace.name(),
                        workspace.path.clone(),
                        cx,
                    )),
            );
        } else {
            pane = pane.child(
                sidebar::navigation_header()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static(
                        "files.git_changes_no_workspace_selected",
                    )),
            );
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
        let view =
            cx.new(|cx| GitDiffDialog::new_document(workspace.clone(), file.clone(), window, cx));
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
                a.forget_workspace_files(&fixture.0.join("beta"));
                assert_eq!(a.active_file, None);
                assert!(fixture.0.join("beta/src/main.rs").exists());
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
