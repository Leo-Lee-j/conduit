//! Contextual file navigation and mixed chat/file documents.
use super::*;
use crate::workspace_files::{self, Entry, FileKey, Stamp};
use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DocumentTab {
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
#[derive(Clone, Copy, Default, Eq, PartialEq)]
enum Navigation {
    #[default]
    Workspaces,
    Files,
    Git,
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
    scroll: gpui_kit::ScrollHandle,
}
struct DiffDocument {
    workspace: PathBuf,
    file: git::ChangedFile,
    view: Entity<GitDiffDialog>,
}
#[derive(Default)]
pub(super) struct FileBrowser {
    pub(super) sidebar_collapsed: bool,
    mode: Navigation,
    explorers: HashMap<PathBuf, Explorer>,
    documents: HashMap<u64, Document>,
    diffs: HashMap<u64, DiffDocument>,
    last_git_refresh: Option<Instant>,
    preview: Option<u64>,
    next_id: u64,
    last_poll: Option<Instant>,
    polling: bool,
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
            .runs
            .iter()
            .filter(|(_, r)| !r.approvals.is_empty())
            .map(|(id, r)| (*id, r.approvals.len()))
            .collect();
        approvals.sort_unstable();
        let mut errors: Vec<_> = app
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
        self.active_diff
            .map(DocumentTab::Diff)
            .or_else(|| self.active_file.map(DocumentTab::File))
            .or_else(|| self.selected_session.map(DocumentTab::Chat))
    }

    pub(super) fn select_document(
        &mut self,
        tab: DocumentTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match tab {
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
                        self.unread_completed.remove(&id);
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
                self.chat_focus.focus(window, cx);
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
                if self.git_snapshot.is_none() && self.git_probe.is_none() {
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
        if closing_active {
            self.active_diff = None;
            self.active_file = None;
            if let Some(next) = self.open_tabs.last().copied() {
                self.select_document(next, window, cx);
            } else if let Some(workspace) = self.selected_workspace {
                self.switch(workspace, None, window, cx);
            } else {
                self.clear_workspace_selection(window, cx);
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
                scroll: gpui_kit::ScrollHandle::new(),
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
        let root = (self.files.mode == Navigation::Files)
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

    pub(super) fn render_navigation(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mode = self.files.mode;
        let pending = self.unread_completed.len() + self.runs.len();
        let rail = v_flex()
            .w(px(38.))
            .h_full()
            .flex_shrink_0()
            .pt_1()
            .gap_1()
            .bg(rgb(palette().sidebar_bg))
            .border_r_1()
            .border_color(rgb(palette().border))
            .child(
                Button::new("nav-workspaces")
                    .debug_selector(|| "nav-workspaces".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(36.))
                    .icon(IconName::Folder)
                    .selected(mode == Navigation::Workspaces)
                    .tooltip(format!("工作区与会话 · {pending} 个运行中或未读任务"))
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.files.mode = Navigation::Workspaces;
                        this.files.sidebar_collapsed = false;
                        cx.notify();
                    })),
            )
            .when(pending > 0, |rail| {
                rail.child(
                    div()
                        .text_size(px(10.))
                        .text_center()
                        .text_color(rgb(palette().highlight_text))
                        .child(pending.to_string()),
                )
            })
            .child(
                Button::new("nav-files")
                    .debug_selector(|| "nav-files".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(36.))
                    .icon(IconName::File)
                    .selected(mode == Navigation::Files)
                    .tooltip("当前工作区文件")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.files.mode = Navigation::Files;
                        this.files.sidebar_collapsed = false;
                        if let Some(i) = this.selected_workspace {
                            this.refresh_tree(this.store.workspaces[i].path.clone(), cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("nav-git")
                    .debug_selector(|| "nav-git".into())
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(36.))
                    .icon(gpui_kit::assets::IconName::GitFork)
                    .selected(mode == Navigation::Git)
                    .tooltip("当前工作区 Git 变更")
                    .accessibility_label("Git 变更")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.show_git_navigation(cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("nav-settings")
                    .debug_selector(|| "nav-settings".into())
                    .accessibility_label("设置")
                    .ghost()
                    .small()
                    .compact()
                    .w(px(36.))
                    .h(px(34.))
                    .icon(IconName::Settings)
                    .tooltip("设置")
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx))),
            );
        let rail = rail.child(
            Button::new("nav-collapse")
                .debug_selector(|| "nav-collapse".into())
                .ghost()
                .small()
                .compact()
                .w(px(36.))
                .h(px(32.))
                .icon(if self.files.sidebar_collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronLeft
                })
                .tooltip("展开 / 收起侧栏")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.files.sidebar_collapsed = !this.files.sidebar_collapsed;
                    cx.notify();
                })),
        );
        if self.files.sidebar_collapsed {
            return rail.into_any_element();
        }
        let pane = match mode {
            Navigation::Workspaces => self.render_sidebar(cx),
            Navigation::Files => self.render_file_tree(cx),
            Navigation::Git => self.render_git_navigation(cx),
        };
        h_flex()
            .id("navigation-panel")
            .debug_selector(|| "navigation-panel".into())
            .size_full()
            .child(rail)
            .child(div().flex_1().min_w_0().h_full().child(pane))
            .into_any_element()
    }

    fn render_file_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(index) = self.selected_workspace else {
            return v_flex()
                .size_full()
                .p_3()
                .gap_3()
                .child("尚未选择工作区")
                .child(
                    Button::new("files-pick-workspace")
                        .label("查看工作区")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.files.mode = Navigation::Workspaces;
                            cx.notify();
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
                    "目录层级超过 64 层".into(),
                ));
                return;
            }
            let Some(directory) = explorer.directories.get(path) else {
                return;
            };
            if directory.loading && directory.entries.is_empty() {
                rows.push(TreeRow::Status(path.to_owned(), depth, "正在加载…".into()));
            } else if let Some(error) = &directory.error {
                rows.push(TreeRow::Status(path.to_owned(), depth, error.clone()));
            } else if directory.entries.is_empty() {
                rows.push(TreeRow::Status(path.to_owned(), depth, "无可见文件".into()));
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
                                    .text_size(px(11.))
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
                                    .pl(px(6. + *depth as f32 * 12.))
                                    .pr_1()
                                    .gap_1()
                                    .items_center()
                                    .cursor_pointer()
                                    .when(active, |row| row.bg(rgb(palette().surface_hover)))
                                    .hover(|s| s.bg(rgb(palette().surface_hover)))
                                    .child(
                                        Icon::new(if directory {
                                            if expanded {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            }
                                        } else {
                                            IconName::File
                                        })
                                        .small()
                                        .text_color(rgb(palette().muted)),
                                    )
                                    .child(div().min_w_0().truncate().text_size(px(12.)).child(
                                        format!("{}{}", entry.name, if link { " ↗" } else { "" }),
                                    ))
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
                                    .into_any_element()
                            }
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .h_full()
        .w_full();
        v_flex()
            .id("file-explorer")
            .debug_selector(|| "file-explorer".into())
            .size_full()
            .min_w_0()
            .bg(rgb(palette().sidebar_bg))
            .child(
                h_flex()
                    .h(px(34.))
                    .flex_shrink_0()
                    .min_w_0()
                    .items_center()
                    .child(div().flex_1().min_w_0().overflow_hidden().child(header))
                    .child(
                        Button::new("files-refresh")
                            .ghost()
                            .small()
                            .compact()
                            .icon(IconName::RefreshCw)
                            .tooltip("刷新文件树")
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.refresh_tree(refresh_root.clone(), cx)
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(list))
            .child(
                Button::new("files-show-ignored")
                    .ghost()
                    .small()
                    .label(if show_ignored {
                        "隐藏忽略文件"
                    } else {
                        "显示忽略文件"
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
            )
            .into_any_element()
    }

    pub(super) fn render_file_document(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(id) = self.active_file else {
            return div().into_any_element();
        };
        let Some(document) = self.files.documents.get(&id) else {
            return div().child("文件标签已关闭").into_any_element();
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
        let markdown = matches!(
            document
                .key
                .relative
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("md" | "markdown" | "mdx")
        );
        let parent_label = parts.join(" / ");
        let text = document.text.clone();
        let copy = text.clone();
        let wrap = document.wrap;
        let status = if document.loading {
            "正在读取…".to_string()
        } else if document.error.is_some() {
            "只读 · 预览不可用".to_string()
        } else {
            format!("只读 · {} · {} 行", document.encoding, text.lines().count())
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
                            .tooltip("返回聊天")
                            .accessibility_label("返回聊天")
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
                            .text_size(px(11.))
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
                            .tooltip(if wrap {
                                "自动换行已开启 · 点击关闭"
                            } else {
                                "开启自动换行"
                            })
                            .accessibility_label("自动换行")
                            .selected(wrap)
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
                            .tooltip("重新读取文件")
                            .accessibility_label("重新读取文件")
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
                            .tooltip("复制文件内容")
                            .accessibility_label("复制文件内容")
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
                    .child(div().font_semibold().child("无法预览此文件"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette().muted))
                            .child(error.clone()),
                    )
                    .child(
                        Button::new(("file-copy-path", id))
                            .debug_selector(|| "file-copy-path".into())
                            .label("复制文件路径")
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
                    .child("空文件"),
            );
        } else {
            let theme = Theme::global(cx);
            page = page.child(
                div()
                    .id(("file-content-scroll", id))
                    .v_flex()
                    .items_start()
                    .debug_selector(|| "file-content-scroll".into())
                    .track_scroll(&document.scroll)
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_scroll()
                    .track_focus(&self.chat_focus)
                    .on_action(cx.listener(
                        |_, _: &gpui_kit::component::input::Copy, window, cx| {
                            let text = TextSelection::selected_text(window, cx);
                            if text.is_empty() {
                                cx.propagate();
                            } else {
                                cx.write_to_clipboard(ClipboardItem::new_string(text));
                            }
                        },
                    ))
                    .on_mouse_up(MouseButton::Left, {
                        let focus = self.chat_focus.clone();
                        move |_, window, cx| focus.focus(window, cx)
                    })
                    .when(!markdown, |area| {
                        area.font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .line_height(px(20.))
                    })
                    .when(markdown, |area| area.p(px(10.)))
                    .child(if markdown {
                        let mut typography = self.chat_typography();
                        if typography.body_font.is_none() {
                            typography.body_font = Some(theme.font_family.clone());
                        }
                        chat_markdown_view(
                            SharedString::from(format!("file-markdown-{id}-{}", document.revision)),
                            text,
                            &typography,
                            &format!("file-markdown-{id}"),
                            true,
                        )
                        .into_any_element()
                    } else {
                        super::numbered_code::file_code(
                            id,
                            document.revision,
                            text,
                            &document.highlights,
                            wrap,
                            cx,
                        )
                        .into_any_element()
                    })
                    .vertical_scrollbar(&document.scroll)
                    .when(!wrap, |area| area.horizontal_scrollbar(&document.scroll)),
            );
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
                .text_size(px(10.))
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
        let mut bar = TabBar::new("session-tabs")
            .folder()
            .with_size(Size::Medium)
            .h(px(36.))
            .when_some(selected, |bar, i| bar.selected_index(i));
        for tab in self.open_tabs.iter().copied() {
            let (title, tooltip, icon, running, unread, selector) = match tab {
                DocumentTab::Chat(id) => {
                    let Some(s) = self.session(id) else {
                        continue;
                    };
                    let title = single_line_title(if s.title.is_empty() {
                        "新会话"
                    } else {
                        &s.title
                    });
                    (
                        title.clone(),
                        title,
                        provider_icon(s.provider).into_any_element(),
                        self.runs.contains_key(&id),
                        self.unread_completed.contains(&id),
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
                        format!(
                            "{} · HEAD → 工作区",
                            d.workspace.join(&d.file.path).display()
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
                .child(
                    div()
                        .max_w(px(180.))
                        .truncate()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .when(preview, |s| s.italic())
                        .child(title),
                );
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
                        .tooltip("关闭标签")
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
            bar = bar.prefix(div().pl_3().text_size(px(12.)).child("选择会话或文件"));
        }
        let app = cx.entity();
        div()
            .h(px(36.))
            .flex_shrink_0()
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
                        PopupMenuItem::new("固定文件标签").on_click(move |_, _, cx| {
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
                    PopupMenuItem::new("关闭右侧标签").on_click(move |_, window, cx| {
                        right.update(cx, |this, cx| {
                            if let Some(i) = this.open_tabs.iter().position(|t| *t == target) {
                                let tabs = this.open_tabs[i + 1..].to_vec();
                                this.close_documents(&tabs, window, cx);
                            }
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("关闭左侧标签").on_click(move |_, window, cx| {
                        left.update(cx, |this, cx| {
                            if let Some(i) = this.open_tabs.iter().position(|t| *t == target) {
                                let tabs = this.open_tabs[..i].to_vec();
                                this.close_documents(&tabs, window, cx);
                            }
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("全部关闭").on_click(move |_, window, cx| {
                        all.update(cx, |this, cx| {
                            let tabs = this.open_tabs.clone();
                            this.close_documents(&tabs, window, cx);
                        });
                    }),
                )
            })
            .into_any_element()
    }
}

impl ConduitApp {
    /// Git is navigation, not a second competing right-side detail dock.
    pub(super) fn git_navigation_open(&self) -> bool {
        self.files.mode == Navigation::Git && !self.files.sidebar_collapsed
    }

    pub(super) fn mark_git_refresh(&mut self) {
        self.files.last_git_refresh = Some(Instant::now());
    }

    pub(super) fn poll_git_navigation(&mut self, cx: &mut Context<Self>) {
        if (self.git_navigation_open() || self.active_diff.is_some())
            && self.git_probe.is_none()
            && self.git_operation.is_none()
            && self.commit_generation.is_none()
            && self
                .files
                .last_git_refresh
                .is_none_or(|time| time.elapsed() >= Duration::from_secs(10))
        {
            self.refresh_git(cx);
        }
    }

    pub(super) fn show_git_navigation(&mut self, cx: &mut Context<Self>) {
        self.files.mode = Navigation::Git;
        self.files.sidebar_collapsed = false;
        self.refresh_git(cx);
        cx.notify();
    }

    fn navigation_workspace_picker(
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
                                    if let Some(id) = session {
                                        this.select_document(DocumentTab::Chat(id), window, cx);
                                    } else {
                                        this.switch(i, None, window, cx);
                                    }
                                    match this.files.mode {
                                        Navigation::Git => this.refresh_git(cx),
                                        Navigation::Files => this.refresh_tree(path.clone(), cx),
                                        Navigation::Workspaces => {}
                                    }
                                }
                            });
                        },
                    ));
                }
                menu
            })
            .into_any_element()
    }

    fn render_git_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let app = cx.entity();
        let mut pane = v_flex()
            .id("git-navigation")
            .debug_selector(|| "git-navigation".into())
            .size_full()
            .min_w_0()
            .bg(rgb(palette().sidebar_bg));
        if let Some(i) = self.selected_workspace {
            let workspace = &self.store.workspaces[i];
            pane = pane.child(
                div()
                    .h(px(34.))
                    .flex_shrink_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(self.navigation_workspace_picker(
                        "git-workspace",
                        workspace.name(),
                        workspace.path.clone(),
                        cx,
                    )),
            );
        } else {
            pane = pane.child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(px(12.))
                    .child("Git 变更 · 未选择工作区"),
            );
        }
        // Both children can shrink: tiny windows scroll the editor instead of cutting off Commit.
        pane.child(
            v_flex()
                .flex_1()
                .min_h(px(72.))
                .min_w_0()
                .px_2()
                .child(self.git_panel(app.clone())),
        )
        .child(
            div()
                .id("git-commit-scroll")
                .max_h(relative(0.5))
                .min_h_0()
                .flex_shrink_1()
                .overflow_y_scroll()
                .border_t_1()
                .border_color(rgb(palette().border))
                .px_2()
                .py_2()
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
            return div().child("差异标签已关闭").into_any_element();
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
                            .text_size(px(11.))
                            .child(document.workspace.display().to_string()),
                    )
                    .child(
                        Button::new("diff-return-chat")
                            .ghost()
                            .small()
                            .label("返回聊天")
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
                    .label(format!("{title} 有 {count} 项待审批，返回聊天处理"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_document(DocumentTab::Chat(id), window, cx)
                    }))
                    .into_any_element(),
            )
        } else {
            self.unread_completed
                .iter()
                .copied()
                .find(|id| {
                    self.session(*id).is_some_and(|s| {
                        s.messages.last().is_some_and(|m| m.kind == ItemKind::Error)
                    })
                })
                .map(|id| {
                    Button::new("document-task-error")
                        .label("后台任务失败，返回聊天查看详情")
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
    use super::{ActiveRun, ChatRuntime, ConduitApp, DocumentTab, FileKey, Store, Workspace};
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
    fn transcript_links_open_preview_and_preserve_chat_context(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let mut store = fixture.store();
        std::fs::write(fixture.0.join("alpha/notes 100%#1.md"), "附件预览").unwrap();
        store.workspaces[0].sessions[0].messages = vec![
            serde_json::from_value(serde_json::json!({
                "key":"link-user", "kind":"User", "text":"查看文件\n\n引用文件：\n- notes 100%#1.md\n"
            })).unwrap(),
            serde_json::from_value(serde_json::json!({
                "key":"link-reply", "kind":"Assistant", "text":"[打开源文件](src/main.rs#L2)"
            }))
            .unwrap(),
        ];
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
                    .update(cx, |input, cx| input.set_value("保留草稿", window, cx));
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(gpui_kit::size(px(1600.), px(800.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let reply = cx
            .debug_bounds("assistant-message-1")
            .expect("painted reply");
        assert!(
            reply.size.width > px(840.),
            "chat fills the available width by default"
        );
        let click = gpui_kit::point(reply.left() + px(24.), reply.top() + px(10.));
        cx.simulate_mouse_move(click, None, Modifiers::default());
        cx.simulate_mouse_down(click, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(click, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let id = app.active_file.expect("link click opens in-app preview");
                assert_eq!(
                    app.files.documents[&id].key,
                    fixture.key("alpha", "src/main.rs")
                );
                assert_eq!(app.files.preview, Some(id));
                assert!(app.open_tabs.contains(&DocumentTab::Chat(1)));
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.input.read(cx).value(), "保留草稿");
                app.select_document(DocumentTab::Chat(1), window, cx);
                assert_eq!(app.input.read(cx).value(), "保留草稿");
                app.open_chat_link(1, "./src/main.rs:2:1", window, cx);
                assert_eq!(app.active_file, Some(id), "same file reuses preview");
                app.open_chat_link(1, "../outside.rs", window, cx);
                assert_eq!(app.active_file, Some(id));
                assert!(app.message.as_ref().unwrap().contains("工作区外部"));
            });
        });
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.select_document(DocumentTab::Chat(1), window, cx)
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let attachment = cx
            .debug_bounds("user-file-0-0")
            .expect("file attachment")
            .center();
        cx.simulate_mouse_move(attachment, None, Modifiers::default());
        cx.simulate_mouse_down(attachment, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(attachment, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|_, cx| {
            let app = app.read(cx);
            assert_eq!(
                app.files.documents[&app.active_file.unwrap()].key,
                fixture.key("alpha", "notes 100%#1.md")
            );
            assert_eq!(app.input.read(cx).value(), "保留草稿");
        });
    }

    #[gpui_kit::test]
    fn file_navigation_preview_pin_close_and_workspace_context(cx: &mut TestAppContext) {
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
        let point = cx.debug_bounds("nav-files").unwrap().center();
        cx.simulate_mouse_move(point, None, Modifiers::default());
        cx.simulate_mouse_down(point, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(point, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("file-explorer").is_some());
        let point = cx.debug_bounds("file-tree-entry-src").unwrap().center();
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
                a.unread_completed.insert(2);
                a.open_workspace_file(fixture.key("beta", "src/main.rs"), true, window, cx);
                assert_eq!(a.selected_workspace, Some(1));
                assert_eq!(a.selected_session, Some(2));
                assert!(a.unread_completed.contains(&2));
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
                a.files.sidebar_collapsed = true;
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

    #[gpui_kit::test]
    fn file_preview_long_unicode_path_and_content_scroll(cx: &mut TestAppContext) {
        use gpui_kit::{Focusable as _, test::TestQueryExt as _};
        use gpui_kit::{ScrollDelta, ScrollWheelEvent, VisualTestContext, point, size};
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
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let relative = Path::new("文档").join("示例 [预览] #1.rs");
        std::fs::create_dir_all(fixture.0.join("alpha/文档")).unwrap();
        let content = (0..500)
            .map(|i| format!("    // 第 {i} 行：{}\n", "long content ".repeat(25)))
            .collect::<String>();
        std::fs::write(fixture.0.join("alpha").join(&relative), content).unwrap();
        let mut store = fixture.store();
        store.workspaces[0].path = store.workspaces[0].path.canonicalize().unwrap();
        let key = FileKey {
            workspace: store.workspaces[0].path.clone(),
            relative,
        };
        let expected_path =
            crate::workspace_files::display_path(&key.workspace.join(&key.relative));
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
                    .update(cx, |input, cx| input.set_value("draft unicode", window, cx));
                app.open_workspace_file(key, false, window, cx);
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(size(px(850.), px(500.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let scroll = cx.update(|_, cx| {
            let a = app.read(cx);
            let d = &a.files.documents[&a.active_file.unwrap()];
            assert!(!d.loading && d.error.is_none());
            d.scroll.clone()
        });
        assert!(scroll.bounds().size.height > px(100.));
        assert!(scroll.bounds().size.height < px(500.));
        assert!(
            scroll.max_offset().y > px(1000.),
            "full source height must overflow: {:?}, {:?}",
            scroll.bounds(),
            scroll.max_offset()
        );
        cx.simulate_event(ScrollWheelEvent {
            position: scroll.bounds().center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
            ..Default::default()
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(scroll.offset().y < px(0.), "wheel must scroll source text");
        assert!(
            scroll.max_offset().x > px(100.),
            "unwrapped lines must overflow horizontally: {:?}",
            scroll.max_offset()
        );
        cx.simulate_event(ScrollWheelEvent {
            position: scroll.bounds().center(),
            delta: ScrollDelta::Pixels(point(px(-100.), px(0.))),
            ..Default::default()
        });
        draw(cx);
        assert!(
            scroll.offset().x < px(0.),
            "horizontal wheel must scroll unwrapped text"
        );
        let toolbar = cx.debug_bounds("file-document-toolbar").unwrap();
        let status = cx.debug_bounds("file-document-status").unwrap();
        let name = cx.debug_bounds("file-path-name").unwrap();
        assert_eq!(toolbar.size.height, px(36.));
        assert_eq!(status.size.height, px(24.));
        assert!(
            name.size.width > px(80.),
            "filename remains visible in a narrow window"
        );
        for selector in ["file-return-chat", "file-wrap", "file-reload", "file-copy"] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert!(
                bounds.size.width <= px(32.),
                "icon-only control must stay compact: {selector} {bounds:?}"
            );
            assert!(bounds.left() >= toolbar.left() && bounds.right() <= toolbar.right());
        }
        cx.update(|window, _| {
            assert_eq!(window.find_all_by_label("自动换行").len(), 1);
            assert_eq!(window.find_all_by_label("返回聊天").len(), 1);
        });
        click(cx, "file-wrap");
        assert_eq!(
            scroll.max_offset().x,
            px(0.),
            "wrapped text fits the viewport"
        );
        assert_eq!(scroll.offset().x, px(0.));
        let before = scroll.offset().y;
        cx.simulate_event(ScrollWheelEvent {
            position: scroll.bounds().center(),
            delta: ScrollDelta::Pixels(point(px(0.), px(-120.))),
            ..Default::default()
        });
        draw(cx);
        assert!(
            scroll.offset().y < before,
            "wrapped text still scrolls vertically"
        );
        click(cx, "file-wrap");
        assert!(scroll.max_offset().x > px(100.));
        let id = cx.update(|_, cx| app.read(cx).active_file.unwrap());
        let before = scroll.offset();
        click(cx, "file-return-chat");
        cx.update(|window, cx| {
            let a = app.read(cx);
            assert_eq!(a.active_file, None);
            assert_eq!(a.selected_session, Some(1));
            assert_eq!(a.input.read(cx).value(), "draft unicode");
            assert!(a.input.read(cx).focus_handle(cx).is_focused(window));
        });
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.select_document(DocumentTab::File(id), window, cx)
            })
        });
        draw(cx);
        assert_eq!(
            scroll.offset(),
            before,
            "switching documents preserves the viewport"
        );
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.files.documents.get_mut(&id).unwrap().error = Some("fixture read error".into());
                cx.notify();
            })
        });
        draw(cx);
        click(cx, "file-copy-path");
        assert_eq!(
            cx.update(|_, cx| cx.read_from_clipboard().and_then(|c| c.text()).unwrap()),
            expected_path
        );
        assert!(!expected_path.starts_with(r"\\?\"));
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn file_refresh_errors_and_background_chat_remain_independent(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let fixture = Fixture::new();
        let store = fixture.store();
        let key = fixture.key("alpha", "src/main.rs");
        let root = key.workspace.clone();
        let (runtime, sender) = ChatRuntime::test_channel();
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
                app.runs.insert(
                    1,
                    ActiveRun::new(
                        runtime,
                        root,
                        "file-background".into(),
                        Default::default(),
                        None,
                        Default::default(),
                    ),
                );
                app.open_workspace_file(key.clone(), false, window, cx);
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.run_until_parked();
        let id = cx.update(|_, cx| app.read(cx).active_file.unwrap());
        cx.update(|_, cx| {
            assert!(app.read(cx).runs.contains_key(&1));
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let frames = cx.update(|_, cx| app.read(cx).file_view.read(cx).render_count);
        sender
            .send(crate::protocol::Event::Delta {
                key: "reply".into(),
                text: "streamed while viewing a file".into(),
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |a, cx| a.poll(window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert_eq!(
            cx.update(|_, cx| app.read(cx).file_view.read(cx).render_count),
            frames,
            "streaming must not reshape the file viewer"
        );
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.runs
                    .get_mut(&1)
                    .unwrap()
                    .approvals
                    .push(crate::protocol::Approval {
                        id: "file-test-approval".into(),
                        title: "command".into(),
                        detail: "test".into(),
                        wire_id: serde_json::json!(1),
                        input: serde_json::Value::Null,
                        allow_once: true,
                        kind: crate::protocol::ApprovalKind::Command,
                    });
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let approval = cx.debug_bounds("file-pending-approval").unwrap().center();
        cx.simulate_mouse_move(approval, None, Modifiers::default());
        cx.simulate_mouse_down(approval, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(approval, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                assert_eq!(a.active_file, None);
                assert_eq!(a.runs[&1].approvals.len(), 1);
                a.runs.get_mut(&1).unwrap().approvals.clear();
                a.open_workspace_file(key.clone(), false, window, cx);
            })
        });
        cx.run_until_parked();
        std::fs::write(key.workspace.join(&key.relative), "fn updated() {}\n").unwrap();
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.files.documents.get_mut(&id).unwrap().stamp = None;
                a.files.last_poll = None;
                a.poll_files(cx);
            })
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert_eq!(a.active_file, Some(id));
            assert_eq!(a.files.documents[&id].text.as_ref(), "fn updated() {}\n");
        });
        sender
            .send(crate::protocol::Event::Finished {
                error: Some("authentication failed".into()),
                cancelled: false,
            })
            .unwrap();
        cx.update(|window, cx| app.update(cx, |a, cx| a.poll(window, cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert_eq!(a.active_file, Some(id));
            assert!(a.unread_completed.contains(&1));
        });
        assert!(cx.debug_bounds("file-document").is_some());
        std::fs::remove_file(key.workspace.join(&key.relative)).unwrap();
        cx.update(|_, cx| app.update(cx, |a, cx| a.load_file(id, cx)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert!(a.files.documents[&id].error.is_some());
            assert_eq!(a.active_file, Some(id));
        });
        std::fs::write(key.workspace.join(&key.relative), "// restored\n").unwrap();
        cx.update(|_, cx| app.update(cx, |a, cx| a.load_file(id, cx)));
        cx.run_until_parked();
        cx.update(|_, cx| assert!(app.read(cx).files.documents[&id].error.is_none()));
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod git_navigation_tests {
    use super::{ConduitApp, DiffMode, DocumentTab, Store, Workspace};
    use crate::git;
    use gpui_kit::{
        AppContext, Entity, Modifiers, MouseButton, TestAppContext, VisualTestContext,
        component::WindowExt as _,
    };
    use gpui_kit::{Focusable as _, test::TestQueryExt as _};
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
                    a.git_probe.is_none() && a.git_operation.is_none() && a.branch_probe.is_none()
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
    fn wait_diff(cx: &mut VisualTestContext, app: &Entity<ConduitApp>, id: u64) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            cx.run_until_parked();
            if cx.update(|_, cx| !app.read(cx).files.diffs[&id].view.read(cx).loading) {
                break;
            }
            assert!(Instant::now() < deadline, "Diff load timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
        draw(cx);
    }

    #[gpui_kit::test]
    fn git_navigation_staging_diff_tabs_and_unique_settings(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let repo = Repo::new();
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
                app.input.update(cx, |input, cx| {
                    input.set_value("keep this draft", window, cx)
                });
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        draw(cx);
        assert!(cx.debug_bounds("nav-settings").is_some());
        assert_eq!(
            cx.update(|window, _| window.find_all_by_label("设置").len()),
            1
        );
        click(cx, "nav-git");
        wait_git(cx, &app);
        assert!(cx.debug_bounds("git-navigation").is_some());
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.ensure_commit_draft(window, cx);
                assert!(!a.details);
                assert_eq!(a.selected_session, Some(1));
                assert_eq!(a.input.read(cx).value(), "keep this draft");
            })
        });
        draw(cx);
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(850.), gpui_kit::px(500.)));
        draw(cx);
        let bounds = cx.update(|window, _| window.find_by_label("Commit").bounds());
        let navigation = cx.debug_bounds("git-navigation").unwrap();
        assert!(
            bounds.left() >= navigation.left()
                && bounds.right() <= navigation.right() + gpui_kit::px(1.),
            "Commit must fit inside the left panel: {bounds:?} vs {navigation:?}"
        );
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(1200.), gpui_kit::px(800.)));
        draw(cx);
        // File rows stage exactly the reviewed changes and stay in the left panel.
        cx.update(|_, cx| {
            app.update(cx, |a, cx| {
                a.stage_git_files(vec!["new file.txt".into()], cx)
            })
        });
        wait_git(cx, &app);
        cx.update(|_, cx| {
            let a = app.read(cx);
            let s = a.git_snapshot.as_ref().unwrap();
            assert_eq!(s.staged_count(), 1);
            assert!(
                s.files
                    .iter()
                    .any(|f| f.path == "src/main.rs" && f.unstaged())
            );
        });
        let file = cx.update(|_, cx| {
            app.read(cx)
                .git_snapshot
                .as_ref()
                .unwrap()
                .files
                .iter()
                .find(|f| f.path == "src/main.rs")
                .unwrap()
                .clone()
        });
        let position = cx
            .debug_bounds("git-change-file-src/main.rs")
            .unwrap()
            .center();
        cx.simulate_mouse_move(position, None, Modifiers::default());
        cx.simulate_event(gpui_kit::MouseDownEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(gpui_kit::MouseUpEvent {
            position,
            button: MouseButton::Left,
            modifiers: Modifiers::default(),
            click_count: 2,
        });
        draw(cx);
        let id = cx.update(|_, cx| app.read(cx).active_diff.unwrap());
        wait_diff(cx, &app, id);
        assert!(cx.debug_bounds("git-diff-document").is_some());
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert_eq!(a.active_file, None);
            assert_eq!(a.selected_session, Some(1));
            assert_eq!(a.input.read(cx).value(), "keep this draft");
            assert!(a.files.diffs[&id].view.read(cx).error.is_none());
            assert!(
                !a.files.diffs[&id]
                    .view
                    .read(cx)
                    .state
                    .read(cx)
                    .files()
                    .is_empty()
            );
        });
        // Reopen deduplicates; unchanged refresh keeps mode, fold state and focus on the other document.
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.open_git_diff(file.clone(), window, cx);
                assert_eq!(a.files.diffs.len(), 1);
                assert_eq!(a.active_diff, Some(id));
            })
        });
        wait_diff(cx, &app, id);
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.files.diffs[&id].view.update(cx, |v, cx| {
                    v.set_mode(DiffMode::Unified, window, cx);
                    v.toggle_unchanged(window, cx);
                });
                a.select_document(DocumentTab::Chat(1), window, cx);
                a.refresh_git(cx);
            })
        });
        wait_git(cx, &app);
        wait_diff(cx, &app, id);
        cx.update(|window, cx| {
            let a = app.read(cx);
            assert_eq!(a.active_diff, None);
            assert!(a.input.read(cx).focus_handle(cx).is_focused(window));
            let v = a.files.diffs[&id].view.read(cx);
            assert_eq!(v.state.read(cx).mode(), DiffMode::Unified);
            assert!(v.unchanged_expanded);
        });
        // Viewing a diff after switching workspaces restores its owning workspace, not the old panel snapshot.
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.switch(1, Some(2), window, cx);
                a.select_document(DocumentTab::Diff(id), window, cx);
                assert_eq!(a.selected_workspace, Some(0));
                assert_eq!(a.selected_session, Some(1));
                assert_eq!(a.input.read(cx).value(), "keep this draft");
            })
        });
        wait_git(cx, &app);
        wait_diff(cx, &app, id);
        // Newly staged and subsequently committed files must not remain a fake added-file diff.
        let new = cx.update(|_, cx| {
            app.read(cx)
                .git_snapshot
                .as_ref()
                .unwrap()
                .files
                .iter()
                .find(|f| f.path == "new file.txt")
                .unwrap()
                .clone()
        });
        cx.update(|window, cx| app.update(cx, |a, cx| a.open_git_diff(new, window, cx)));
        let new_id = cx.update(|_, cx| app.read(cx).active_diff.unwrap());
        wait_diff(cx, &app, new_id);
        repo.git(&["commit", "-qm", "add new file"]);
        cx.update(|_, cx| app.update(cx, |a, cx| a.refresh_git(cx)));
        wait_git(cx, &app);
        wait_diff(cx, &app, new_id);
        cx.update(|_, cx| {
            assert!(
                app.read(cx).files.diffs[&new_id]
                    .view
                    .read(cx)
                    .state
                    .read(cx)
                    .files()
                    .is_empty()
            )
        });
        // Escape only closes the active diff; it cannot close a chat or cancel a model task.
        cx.simulate_keystrokes("escape");
        draw(cx);
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert!(!a.files.diffs.contains_key(&new_id));
            assert!(a.open_tabs.contains(&DocumentTab::Chat(1)));
        });
        // Placeholder bodies must retain keyboard focus, and recovery with the same
        // patch must hand focus back to the visible DiffState instead of stealing chat focus.
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                a.select_document(DocumentTab::Diff(id), window, cx)
            });
        });
        wait_diff(cx, &app, id);
        cx.update(|window, cx| {
            let view = app.read(cx).files.diffs[&id].view.clone();
            view.update(cx, |v, cx| {
                v.finish_load(
                    Ok(git::FileDiff {
                        patch: "--- /dev/null\n+++ b/image.bin\nBinary files /dev/null and b/image.bin differ\n".into(),
                        notes: Vec::new(),
                    }),
                    window,
                    cx,
                );
                assert!(v.error.is_none());
                assert!(v.state.read(cx).files()[0].is_binary());
                assert!(!v.state.read(cx).focus_handle(cx).is_focused(window));
            });
        });
        draw(cx);
        cx.update(|window, cx| {
            let view = app.read(cx).files.diffs[&id].view.clone();
            view.update(cx, |v, cx| v.reload(window, cx));
        });
        wait_diff(cx, &app, id);
        cx.update(|window, cx| {
            let view = app.read(cx).files.diffs[&id].view.clone();
            view.update(cx, |v, cx| {
                assert!(v.state.read(cx).focus_handle(cx).is_focused(window));
                v.finish_load(Err(anyhow::anyhow!("fixture read failed")), window, cx);
                assert!(!v.state.read(cx).focus_handle(cx).is_focused(window));
            });
        });
        draw(cx);
        cx.update(|window, cx| {
            let view = app.read(cx).files.diffs[&id].view.clone();
            view.update(cx, |v, cx| v.reload(window, cx));
        });
        wait_diff(cx, &app, id);
        cx.update(|window, cx| {
            let view = app.read(cx).files.diffs[&id].view.clone();
            view.update(cx, |v, cx| {
                assert!(v.error.is_none());
                assert!(v.state.read(cx).focus_handle(cx).is_focused(window));
                v.finish_load(Err(anyhow::anyhow!("fixture read failed")), window, cx);
            });
        });
        draw(cx);
        cx.simulate_keystrokes("ctrl-w");
        draw(cx);
        cx.update(|_, cx| {
            let a = app.read(cx);
            assert!(!a.files.diffs.contains_key(&id));
            assert!(a.open_tabs.contains(&DocumentTab::Chat(1)));
        });
        cx.update(|window, cx| {
            app.update(cx, |a, cx| {
                let tabs = a.open_tabs.clone();
                a.close_documents(&tabs, window, cx);
                assert!(a.open_tabs.is_empty());
                assert_eq!(a.active_diff, None);
                assert_eq!(a.selected_session, None);
                assert_eq!(a.drafts.get(&1).unwrap(), "keep this draft");
                let _ = window;
            })
        });
        draw(cx);
        click(cx, "nav-settings");
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        cx.update(|window, cx| window.close_dialog(cx));
        app.update(cx, |a, _| a.writable = false);
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(app);
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn start_in_git_opens_left_navigation_without_right_dock(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let app = ConduitApp::with_store(
                    window,
                    cx,
                    Ok(Store {
                        version: 2,
                        ..Default::default()
                    }),
                    false,
                    true,
                );
                assert!(app.git_navigation_open());
                assert!(!app.details);
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
        draw(cx);
        assert!(cx.debug_bounds("git-navigation").is_some());
        assert!(cx.debug_bounds("nav-settings").is_some());
        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
    }
}
