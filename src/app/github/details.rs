//! File navigation and embedded, remote-revision diff for a request document.
use super::*;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use std::collections::{BTreeMap, HashSet};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[derive(Clone)]
struct TreeRow {
    path: String,
    label: String,
    depth: usize,
    file: Option<usize>,
    count: usize,
}

#[derive(Default)]
struct Directory {
    path: String,
    label: String,
    directories: BTreeMap<String, Directory>,
    files: Vec<usize>,
    count: usize,
}

fn tree_rows(files: &[crate::github::details::File]) -> Vec<TreeRow> {
    let mut root = Directory::default();
    for (index, file) in files.iter().enumerate() {
        let mut directory = &mut root;
        directory.count += 1;
        let mut path = String::new();
        let mut parts = file.path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                break;
            }
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(part);
            directory = directory
                .directories
                .entry(part.into())
                .or_insert_with(|| Directory {
                    path: path.clone(),
                    label: part.into(),
                    ..Default::default()
                });
            directory.count += 1;
        }
        directory.files.push(index);
    }
    fn flatten(
        directory: &Directory,
        files: &[crate::github::details::File],
        depth: usize,
        rows: &mut Vec<TreeRow>,
    ) {
        for child in directory.directories.values() {
            let mut child = child;
            let mut label = child.label.clone();
            while child.files.is_empty() && child.directories.len() == 1 {
                child = child.directories.values().next().unwrap();
                label.push('/');
                label.push_str(&child.label);
            }
            rows.push(TreeRow {
                path: child.path.clone(),
                label,
                depth,
                file: None,
                count: child.count,
            });
            flatten(child, files, depth + 1, rows);
        }
        for &index in &directory.files {
            let file = &files[index];
            rows.push(TreeRow {
                path: file.path.clone(),
                label: file.path.rsplit('/').next().unwrap_or(&file.path).into(),
                depth,
                file: Some(index),
                count: 1,
            });
        }
    }
    let mut rows = Vec::new();
    flatten(&root, files, 0, &mut rows);
    rows
}

pub(super) struct DetailsUi {
    pub snapshot: Option<Arc<crate::github::details::Snapshot>>,
    rows: Vec<TreeRow>,
    collapsed: HashSet<String>,
    selected: Option<String>,
    opened: Option<String>,
    diff: Option<Entity<super::super::git_diff::GitDiffDialog>>,
    diff_source: Option<String>,
    input: Option<Entity<InputState>>,
    input_subscription: Option<Subscription>,
    scroll: UniformListScrollHandle,
    pub loading: bool,
    pub error: Option<String>,
    cancel: Arc<AtomicBool>,
    task: Option<Task<()>>,
    pub composers: HashMap<String, Entity<GithubCommentPanel>>,
    width: f32,
    files_width: f32,
    discussion_width: f32,
    focused: bool,
    show_narrow_discussion: bool,
}

impl Default for DetailsUi {
    fn default() -> Self {
        Self {
            snapshot: None,
            rows: Vec::new(),
            collapsed: HashSet::new(),
            selected: None,
            opened: None,
            diff: None,
            diff_source: None,
            input: None,
            input_subscription: None,
            scroll: Default::default(),
            loading: false,
            error: None,
            cancel: Arc::new(AtomicBool::new(false)),
            task: None,
            composers: HashMap::new(),
            width: 0.,
            files_width: 280.,
            discussion_width: 380.,
            focused: false,
            show_narrow_discussion: false,
        }
    }
}

impl Drop for DetailsUi {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl DetailsUi {
    pub fn install_snapshot(&mut self, snapshot: crate::github::details::Snapshot) {
        self.rows = tree_rows(&snapshot.files);
        self.snapshot = Some(Arc::new(snapshot));
    }

    pub fn cancel_load(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.task = None;
        self.loading = false;
    }

    pub fn reset_request(&mut self) {
        self.cancel_load();
        self.snapshot = None;
        self.rows.clear();
        self.collapsed.clear();
        self.selected = None;
        self.opened = None;
        self.diff = None;
        self.diff_source = None;
        self.error = None;
        self.focused = false;
        self.show_narrow_discussion = false;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }
}

impl PullRequestPanel {
    pub(in crate::app) fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        let forms: Vec<_> = self.details.composers.values().cloned().collect();
        for form in forms {
            let allowed = form.update(cx, |form, cx| form.can_close(cx));
            if !allowed {
                let expected = form.read(cx).expected.clone();
                if self
                    .snapshot
                    .as_ref()
                    .is_none_or(|pr| pr.url != expected.url)
                {
                    self.cancel.store(true, Ordering::Release);
                    self.loading = false;
                    self.details.reset_request();
                    self.requested = Some((expected.forge, expected.number, expected.url.clone()));
                    self.snapshot = Some(expected);
                }
                self.tab = 2;
                self.details.focused = false;
                self.details.show_narrow_discussion = true;
                cx.notify();
                return false;
            }
        }
        true
    }

    pub(super) fn load_details(&mut self, cx: &mut Context<Self>) {
        if !self.document {
            return;
        }
        let Some(pr) = self.snapshot.clone() else {
            return;
        };
        self.details.cancel_load();
        if self
            .details
            .snapshot
            .as_ref()
            .is_some_and(|old| old.pr != pr.url || old.head != pr.head)
        {
            self.details.reset_request();
        }
        self.details.cancel = Arc::new(AtomicBool::new(false));
        self.details.loading = true;
        self.details.error = None;
        let workspace = self.workspace.clone();
        let cancel = self.details.cancel.clone();
        self.details.task = Some(cx.spawn(async move |this, cx| {
            let ticket = cancel.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::github::details::load(&workspace, &pr, &cancel)
                        .map_err(sign_in::describe)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if ticket.load(Ordering::Acquire) || !Arc::ptr_eq(&ticket, &this.details.cancel) {
                    return;
                }
                this.details.loading = false;
                match result {
                    Ok(snapshot) => {
                        this.details.install_snapshot(snapshot);
                    }
                    Err((error, required)) => {
                        this.details.error = Some(error);
                        if required.is_some() {
                            this.sign_in.set(required);
                        }
                    }
                }
                cx.notify();
            });
        }));
    }

    fn ensure_file_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.details.input.is_some() {
            return;
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(label("github.detail_search")));
        self.details.input_subscription =
            Some(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.details.scroll.scroll_to_item(0, ScrollStrategy::Top);
                    cx.notify();
                }
            }));
        self.details.input = Some(input);
    }

    fn visible_file_rows(&self, cx: &App) -> Vec<usize> {
        let query = self
            .details
            .input
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_lowercase())
            .unwrap_or_default();
        let mut matching = HashSet::new();
        let mut parents = HashSet::new();
        if !query.is_empty() {
            for row in &self.details.rows {
                if row.file.is_some() && row.path.to_lowercase().contains(&query) {
                    matching.insert(row.path.as_str());
                    let mut parent = row.path.as_str();
                    while let Some((prefix, _)) = parent.rsplit_once('/') {
                        parents.insert(prefix);
                        parent = prefix;
                    }
                }
            }
        }
        let mut hidden_depth = None;
        self.details
            .rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                if query.is_empty() {
                    if hidden_depth.is_some_and(|depth| row.depth > depth) {
                        return None;
                    }
                    hidden_depth = None;
                    if row.file.is_none() && self.details.collapsed.contains(&row.path) {
                        hidden_depth = Some(row.depth);
                    }
                } else if !(matching.contains(row.path.as_str())
                    || parents.contains(row.path.as_str()))
                {
                    return None;
                }
                Some(index)
            })
            .collect()
    }

    fn ensure_request_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), Some(snapshot), Some(pr)) =
            (&self.details.opened, &self.details.snapshot, &self.snapshot)
        else {
            return;
        };
        if snapshot.pr != pr.url || snapshot.head != pr.head {
            return;
        }
        let Some(file) = snapshot.files.iter().find(|file| &file.path == path) else {
            self.details.opened = None;
            self.details.diff = None;
            self.details.diff_source = None;
            return;
        };
        let source = format!(
            "request:{}:{}:{}:{}",
            snapshot.pr, snapshot.comparison, snapshot.head, file.path
        );
        if self.details.diff_source.as_ref() == Some(&source) {
            return;
        }
        let file = file.clone();
        let versions = (
            format!("{} · {}", pr.base, label("github.detail_base")),
            format!(
                "{} · {}",
                pr.branch,
                pr.head.chars().take(12).collect::<String>()
            ),
        );
        let workspace = self.workspace.clone();
        let mode = if self.details.width - self.details.files_width - self.details.discussion_width
            < 700.
        {
            DiffMode::Unified
        } else {
            DiffMode::Split
        };
        let view = cx.new(|cx| {
            let view = super::super::git_diff::GitDiffDialog::from_remote(
                workspace,
                file.changed_file(),
                source.clone(),
                versions,
                file.diff(),
                window,
                cx,
            );
            view.state.update(cx, |state, cx| state.set_mode(mode, cx));
            view
        });
        self.details.diff = Some(view);
        self.details.diff_source = Some(source);
    }

    pub(super) fn open_request_file(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading || self.details.loading {
            return;
        }
        let Some(snapshot) = &self.details.snapshot else {
            return;
        };
        let Some(file) = snapshot
            .files
            .iter()
            .find(|file| file.path == path || file.original.as_deref() == Some(path))
        else {
            return;
        };
        self.details.selected = Some(file.path.clone());
        self.details.opened = Some(file.path.clone());
        self.details.show_narrow_discussion = false;
        self.ensure_request_diff(window, cx);
        if let Some(view) = &self.details.diff {
            view.update(cx, |view, cx| view.focus_diff(window, cx));
        }
        cx.notify();
    }

    fn file_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let row = &self.details.rows[index];
        let path = row.path.clone();
        let is_file = row.file.is_some();
        let status = row.file.and_then(|index| {
            self.details
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.files[index].status)
        });
        let selected = self.details.selected.as_deref() == Some(&row.path);
        let icon = if is_file {
            file_ref::icon_for(Path::new(&row.path))
        } else {
            FullIconName::Folder
        };
        let mut content = h_flex()
            .min_w_0()
            .w_full()
            .gap_1()
            .items_center()
            .child(Icon::new(icon).small())
            .child(div().flex_1().min_w_0().truncate().child(row.label.clone()));
        if let Some(status) = status {
            content = content.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(match status {
                        'A' => palette().git_added_text,
                        'D' => palette().git_deleted_text,
                        _ => palette().git_modified_text,
                    }))
                    .child(status.to_string()),
            );
        } else {
            content = content.child(
                div()
                    .text_size(ui_text_size(10.))
                    .text_color(rgb(palette().muted))
                    .child(row.count.to_string()),
            );
        }
        Button::new(("github-file", index))
            .debug_selector(move || format!("github-file-{index}"))
            .small()
            .ghost()
            .selected(selected)
            .w_full()
            .h(px(28.))
            .pl(px(6. + row.depth as f32 * 12.))
            .tooltip(row.path.clone())
            .accessibility_label(row.path.clone())
            .cursor_pointer()
            .disabled(is_file && (self.loading || self.details.loading))
            .child(content)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if is_file {
                    this.details.selected = Some(path.clone());
                    if event.click_count() == 2 || matches!(event, ClickEvent::Keyboard(_)) {
                        this.open_request_file(&path, window, cx);
                    }
                } else if !this.details.collapsed.remove(&path) {
                    this.details.collapsed.insert(path.clone());
                }
                cx.notify();
            }))
            .into_any_element()
    }

    pub(super) fn detail_description(&self) -> AnyElement {
        if self.details.loading && self.details.snapshot.is_none() {
            return content_skeleton("github-description-skeleton");
        }
        let Some(snapshot) = &self.details.snapshot else {
            return div().into_any_element();
        };
        let date = chrono::DateTime::parse_from_rfc3339(&snapshot.created_at)
            .map(|date| {
                date.with_timezone(&Local)
                    .format("%Y/%m/%d %H:%M")
                    .to_string()
            })
            .unwrap_or_default();
        v_flex()
            .gap_2()
            .pb_3()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .child(div().font_semibold().child(snapshot.author.clone()))
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(date),
                    ),
            )
            .child(
                TextView::markdown(
                    "github-request-description",
                    if snapshot.description.is_empty() {
                        label("github.detail_no_description").to_owned()
                    } else {
                        snapshot.description.clone()
                    },
                )
                .selectable(true)
                .text_size(ui_text_size(13.)),
            )
            .into_any_element()
    }

    pub(super) fn document_layout(
        &mut self,
        controls: AnyElement,
        body: AnyElement,
        tabs: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_file_input(window, cx);
        self.ensure_request_diff(window, cx);
        let mut files = v_flex()
            .id("github-files-column")
            .debug_selector(|| "github-files-column".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(palette().sidebar_bg));
        if self.loading && self.snapshot.is_none() {
            files = files.child(content_skeleton("github-metadata-skeleton"));
        } else if let Some(pr) = &self.snapshot {
            let mut meta = v_flex()
                .p_3()
                .gap_2()
                .child(
                    div()
                        .text_size(ui_text_size(15.))
                        .font_semibold()
                        .child(format!(
                            "{}  {}",
                            pr.title,
                            pr.forge.request_name(pr.number)
                        )),
                )
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .child(presentation::pr_state(&pr.state)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(format!("{} → {}", pr.branch, pr.base)),
                );
            let checks = pr
                .checks
                .iter()
                .filter(|check| check.needs_attention())
                .count();
            meta = meta.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(if checks > 0 {
                        palette().warning_text
                    } else {
                        palette().muted
                    }))
                    .child(conduit::i18n::format(
                        "github.detail_check_count",
                        &[("count", checks.to_string())],
                    )),
            );
            files = files.child(meta);
        }
        files = files.child(div().px_3().pb_2().child(controls)).child(
            v_flex()
                .p_3()
                .gap_2()
                .border_t_1()
                .border_color(rgb(palette().border))
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .child(div().font_semibold().child(label("github.detail_files")))
                        .when_some(self.details.snapshot.as_ref(), |row, snapshot| {
                            row.child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::format(
                                        "github.detail_file_count",
                                        &[("count", snapshot.files.len().to_string())],
                                    )),
                            )
                            .when_some(
                                snapshot.commits,
                                |row, commits| {
                                    row.child(
                                        div()
                                            .text_size(ui_text_size(11.))
                                            .text_color(rgb(palette().muted))
                                            .child(conduit::i18n::format(
                                                "github.detail_commits",
                                                &[("count", commits.to_string())],
                                            )),
                                    )
                                },
                            )
                        }),
                )
                .child(
                    Input::new(self.details.input.as_ref().unwrap())
                        .h(px(28.))
                        .focus_bordered(false),
                ),
        );
        if let Some(error) = &self.details.error {
            files = files.child(
                v_flex()
                    .px_3()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().warning_text))
                            .child(error.clone()),
                    )
                    .child(
                        Button::new("github-files-retry")
                            .small()
                            .ghost()
                            .label(label("github.refresh"))
                            .disabled(self.loading || self.details.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    ),
            );
        }
        if self
            .details
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.limited)
        {
            files = files.child(
                div()
                    .px_3()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().warning_text))
                    .child(label("github.detail_files_limited")),
            );
        }
        let indices = self.visible_file_rows(cx);
        if self.loading || self.details.loading {
            files = files.child(
                v_flex()
                    .id("github-files-skeleton")
                    .debug_selector(|| "github-files-skeleton".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .p_3()
                    .gap_2()
                    .children([0.76, 0.9, 0.68, 0.84, 0.72, 0.88].map(|width| {
                        h_flex()
                            .h(px(22.))
                            .gap_2()
                            .items_center()
                            .child(Skeleton::new().size(px(12.)).rounded_sm())
                            .child(
                                Skeleton::new()
                                    .w(relative(width))
                                    .h(px(12.))
                                    .rounded_sm()
                                    .secondary(),
                            )
                    })),
            );
        } else if indices.is_empty() {
            files = files
                .when(self.details.error.is_none(), |files| {
                    files.child(
                        div()
                            .p_3()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(label("github.detail_files_empty")),
                    )
                })
                .child(div().flex_1());
        } else {
            files = files.child(
                uniform_list(
                    "github-file-tree",
                    indices.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| this.file_row(indices[index], cx))
                            .collect()
                    }),
                )
                .flex_1()
                .min_h_0()
                .track_scroll(&self.details.scroll),
            );
        }
        files = files.child(
            v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .flex_shrink_0()
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(label("github.detail_open_hint")),
                )
                .when_some(self.details.snapshot.as_ref(), |column, snapshot| {
                    column.child(
                        div()
                            .text_size(ui_text_size(11.))
                            .text_color(rgb(palette().muted))
                            .child(if snapshot.reviewers.is_empty() {
                                label("github.detail_no_reviewer").to_owned()
                            } else {
                                format!(
                                    "{}: {}",
                                    label("github.detail_reviewers"),
                                    snapshot.reviewers.join(", ")
                                )
                            }),
                    )
                }),
        );
        files = files.child(
            document_footer()
                .id("github-files-footer")
                .debug_selector(|| "github-files-footer".into())
                .child(
                    Button::new("github-show-review")
                        .small()
                        .outline()
                        .h(px(28.))
                        .icon(FullIconName::MessageSquare)
                        .label(label("github.threads"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tab = 1;
                            this.details.focused = false;
                            this.details.show_narrow_discussion = true;
                            cx.notify();
                        })),
                ),
        );
        let has_diff = self.details.diff.is_some();
        let narrow = self.details.width > 0. && self.details.width < 960.;
        let show_diff =
            has_diff && (!narrow || self.details.focused || !self.details.show_narrow_discussion);
        let show_discussion = !show_diff || (!self.details.focused && !narrow);
        let mut middle = v_flex()
            .id("github-discussion-column")
            .debug_selector(|| "github-discussion-column".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .items_center()
                    .child(div().flex_1().child(label("github.detail_activity")))
                    .when(has_diff, |row| {
                        row.child(
                            Button::new("github-focus-diff")
                                .small()
                                .ghost()
                                .label(label("github.detail_focus"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details.focused = true;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when_some(tabs, |column, tabs| column.child(div().px_3().child(tabs)))
            .child(v_flex().flex_1().min_h_0().min_w_0().px_3().child(body));
        if let Some(composer) = self
            .snapshot
            .as_ref()
            .and_then(|pr| self.details.composers.get(&pr.url))
        {
            middle = middle.child(
                div()
                    .id("github-inline-composer")
                    .w_full()
                    .flex_shrink_0()
                    .child(composer.clone()),
            );
        } else {
            middle = middle.child(
                document_footer()
                    .id("github-discussion-footer")
                    .debug_selector(|| "github-discussion-footer".into())
                    .justify_end()
                    .child(
                        Button::new("github-publish-comment")
                            .small()
                            .primary()
                            .h(px(28.))
                            .icon(FullIconName::MessageSquarePlus)
                            .label(label("github.publish_comment"))
                            .disabled(
                                self.loading
                                    || self.error.is_some()
                                    || self.snapshot.as_ref().is_none_or(|pr| pr.state != "OPEN")
                                    || self.host.upgrade().is_none_or(|host| {
                                        let app = host.read(cx);
                                        !app.writable || app.closing
                                    }),
                            )
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_comment(window, cx)),
                            ),
                    ),
            );
        }
        let layout: usize = if show_diff {
            if show_discussion { 3 } else { 2 }
        } else {
            0
        };
        let panel = cx.weak_entity();
        let files_max = if self.details.width > 0. {
            (self.details.width - if show_diff { 200. } else { 240. }).clamp(170., 420.)
        } else {
            420.
        };
        let mut group = h_resizable(("github-request-panes", layout))
            .child(
                resizable_panel()
                    .size(px(self.details.files_width))
                    .size_range(px(170.)..px(files_max))
                    .fixed()
                    .child(files),
            )
            .on_resize(move |state, _, cx| {
                let sizes = state.read(cx).sizes().clone();
                let _ = panel.update(cx, |this, _| {
                    if let Some(size) = sizes.first() {
                        this.details.files_width = f32::from(*size);
                    }
                    if layout == 3
                        && let Some(size) = sizes.get(1)
                    {
                        this.details.discussion_width = f32::from(*size);
                    }
                });
            });
        if show_discussion {
            group = group.child(if show_diff {
                resizable_panel()
                    .size(px(self.details.discussion_width))
                    .size_range(px(240.)..px(700.))
                    .child(middle)
            } else {
                resizable_panel()
                    .size_range(px(240.)..px(10_000.))
                    .child(middle)
            });
        }
        if show_diff && let Some(view) = &self.details.diff {
            let diff = v_flex()
                .id("github-diff-column")
                .debug_selector(|| "github-diff-column".into())
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(
                    h_flex()
                        .px_2()
                        .py_1()
                        .gap_1()
                        .justify_end()
                        .child(
                            Button::new("github-restore-discussion")
                                .small()
                                .ghost()
                                .label(label(if show_discussion {
                                    "github.detail_focus"
                                } else {
                                    "github.detail_show_activity"
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.details.focused = show_discussion;
                                    this.details.show_narrow_discussion = !show_discussion;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("github-close-diff")
                                .small()
                                .ghost()
                                .icon(FullIconName::X)
                                .tooltip(label("github.detail_close_diff"))
                                .accessibility_label(label("github.detail_close_diff"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details.opened = None;
                                    this.details.diff = None;
                                    this.details.diff_source = None;
                                    this.details.focused = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex_1().min_h_0().min_w_0().child(view.clone()));
            group = group.child(
                resizable_panel()
                    .size_range(px(200.)..px(10_000.))
                    .child(diff),
            );
        }
        let measure = cx.weak_entity();
        let host = self.host.clone();
        let workspace = self.workspace.clone();
        v_flex()
            .id("github-document")
            .debug_selector(|| "github-document".into())
            .key_context("PullRequestDocument")
            .track_focus(&self.focus)
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(group)
            .on_prepaint(move |bounds, window, cx| {
                let width = f32::from(bounds.size.width);
                let Some(panel) = measure.upgrade() else {
                    return;
                };
                if (panel.read(cx).details.width - width).abs() < 1. {
                    return;
                }
                let changed = panel.update(cx, |this, _| {
                    let before = this.details.width > 0. && this.details.width < 960.;
                    this.details.width = width;
                    before != (width < 960.)
                });
                if changed {
                    window.defer(cx, move |_, cx| panel.update(cx, |_, cx| cx.notify()));
                }
            })
            .on_action(move |_: &CloseDocument, window, cx| {
                if let Some(host) = host.upgrade() {
                    host.update(cx, |app, cx| {
                        if let Some(id) = app.github_ui.id_for(&workspace) {
                            app.close_documents(&[files::DocumentTab::PullRequest(id)], window, cx);
                        }
                    });
                }
            })
            .into_any_element()
    }
}
