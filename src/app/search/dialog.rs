//! One search surface for session history and workspace filenames.
use super::*;
use crate::{
    file_search::{self, FileIndex},
    workspace_files::FileKey,
};
use gpui_kit::component::{kbd::Kbd, skeleton::Skeleton};

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

actions!(
    workspace_search,
    [PreviousResult, NextResult, OpenResult, CloseSearch]
);

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Category {
    All,
    Sessions,
    Files,
}
impl Category {
    fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::All => "search.all",
            Self::Sessions => "search.sessions",
            Self::Files => "search.files",
        })
    }
}

#[derive(Clone)]
enum ResultItem {
    Session(SearchHit),
    File {
        key: FileKey,
        workspace_name: String,
    },
}
impl ResultItem {
    fn title(&self) -> String {
        match self {
            Self::Session(hit) => hit.title.clone(),
            Self::File { key, .. } => key
                .relative
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        }
    }
    fn icon(&self) -> FullIconName {
        match self {
            Self::Session(_) => FullIconName::MessagesSquare,
            Self::File { .. } => FullIconName::File,
        }
    }
}

struct WorkspaceSearch {
    app: WeakEntity<ConduitApp>,
    input: Entity<InputState>,
    workspaces: Vec<(PathBuf, String)>,
    scope: Option<PathBuf>,
    category: Category,
    indexes: BTreeMap<PathBuf, Arc<FileIndex>>,
    scanning: HashSet<PathBuf>,
    results: Vec<ResultItem>,
    all_results: Vec<ResultItem>,
    counts: (usize, usize),
    selected: usize,
    searching: bool,
    generation: u64,
    cancel: Arc<AtomicBool>,
    task: Option<Task<()>>,
    errors: usize,
    scroll: ScrollHandle,
    _input_subscription: Subscription,
}

impl Drop for WorkspaceSearch {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl WorkspaceSearch {
    fn new(
        app: WeakEntity<ConduitApp>,
        category: Category,
        workspaces: Vec<(PathBuf, String)>,
        scope: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        for context in ["WorkspaceSearch", "WorkspaceSearch > Input"] {
            cx.bind_keys([
                KeyBinding::new("up", PreviousResult, Some(context)),
                KeyBinding::new("down", NextResult, Some(context)),
                KeyBinding::new("enter", OpenResult, Some(context)),
                KeyBinding::new("escape", CloseSearch, Some(context)),
            ]);
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("search.everything_placeholder"))
        });
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refresh(cx);
            }
        });
        let search = Self {
            app,
            input,
            workspaces,
            scope,
            category,
            indexes: BTreeMap::new(),
            scanning: HashSet::new(),
            results: Vec::new(),
            all_results: Vec::new(),
            counts: (0, 0),
            selected: 0,
            searching: false,
            generation: 0,
            cancel: Arc::default(),
            task: None,
            errors: 0,
            scroll: ScrollHandle::new(),
            _input_subscription: subscription,
        };
        let entity = cx.entity();
        cx.defer(move |cx| entity.update(cx, |search, cx| search.refresh(cx)));
        search
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.generation = self.generation.wrapping_add(1);
        self.results.clear();
        self.all_results.clear();
        self.counts = (0, 0);
        self.selected = 0;
        self.searching = true;
        self.errors = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
        let scope = self.scope.clone();
        for (root, _) in &self.workspaces {
            if scope.as_ref().is_some_and(|scope| scope != root)
                || self.indexes.contains_key(root)
                || !self.scanning.insert(root.clone())
            {
                continue;
            }
            let root = root.clone();
            cx.spawn(async move |this, cx| {
                let scan_root = root.clone();
                let index = cx
                    .background_executor()
                    .spawn(async move { file_search::discover(&scan_root) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    this.scanning.remove(&root);
                    this.indexes.insert(root, Arc::new(index));
                    this.refresh(cx);
                });
            })
            .detach();
        }
        let Ok(workspaces) = self.app.update(cx, |app, _| {
            app.store
                .workspaces
                .iter()
                .filter(|w| scope.as_ref().is_none_or(|scope| scope == &w.path))
                .cloned()
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        let query = self.input.read(cx).value().trim().to_lowercase();
        let indexes = self.indexes.clone();
        let generation = self.generation;
        let cancel = self.cancel.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            if cancel.load(Ordering::Acquire) {
                return;
            }
            let (mut sessions, mut files, errors, versions) = cx
                .background_executor()
                .spawn(async move {
                    let mut files = Vec::new();
                    let mut errors = 0;
                    let mut versions = HashMap::new();
                    for workspace in &workspaces {
                        if cancel.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(index) = indexes.get(&workspace.path) {
                            errors += usize::from(index.error.is_some());
                            for relative in file_search::search_with_limit(index, &query, 50) {
                                files.push(ResultItem::File {
                                    key: FileKey {
                                        workspace: workspace.path.clone(),
                                        relative: relative.into(),
                                    },
                                    workspace_name: workspace.name(),
                                });
                                if files.len() == 50 {
                                    break;
                                }
                            }
                        }
                        if files.len() == 50 {
                            break;
                        }
                    }
                    let sessions = if query.is_empty() {
                        let mut recent: Vec<_> = workspaces
                            .iter()
                            .flat_map(|w| {
                                w.visible_sessions().filter(|s| !s.archived).map(|s| {
                                    (
                                        s.last_activity_at(),
                                        SearchHit {
                                            session: s.id,
                                            provider: s.provider,
                                            workspace_name: w.name(),
                                            title: if s.title.is_empty() {
                                                conduit::i18n::text_static("files.new_session")
                                                    .into()
                                            } else {
                                                s.title.clone()
                                            },
                                            archived: s.archived,
                                            snippet: String::new(),
                                        },
                                    )
                                })
                            })
                            .collect();
                        recent.sort_by_key(|(activity, hit)| {
                            std::cmp::Reverse((*activity, hit.session))
                        });
                        recent.into_iter().take(40).map(|(_, hit)| hit).collect()
                    } else {
                        let (results, _) = super::super::search_worker::collect_cached(
                            &workspaces,
                            &query,
                            &cancel,
                            Default::default(),
                        );
                        errors += results.errors;
                        versions = results.versions;
                        results.hits
                    };
                    (sessions, files, errors, versions)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let _ = this.app.update(cx, |app, _| {
                    sessions.retain(|hit| {
                        app.session(hit.session).is_some_and(|session| {
                            !session.is_subagent
                                && versions.get(&hit.session).map_or(
                                    session.title == hit.title || session.title.is_empty(),
                                    |version| version.matches(session),
                                )
                        })
                    });
                    files.retain(|item| match item {
                        ResultItem::File { key, .. } => {
                            app.store.workspaces.iter().any(|w| w.path == key.workspace)
                        }
                        _ => false,
                    });
                });
                this.counts = (sessions.len(), files.len());
                this.all_results = sessions
                    .into_iter()
                    .map(ResultItem::Session)
                    .chain(files)
                    .collect();
                this.filter_category();
                this.searching = false;
                this.errors = errors;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn filter_category(&mut self) {
        self.results = self
            .all_results
            .iter()
            .filter(|item| {
                matches!(
                    (self.category, item),
                    (Category::All, _)
                        | (Category::Sessions, ResultItem::Session(_))
                        | (Category::Files, ResultItem::File { .. })
                )
            })
            .cloned()
            .collect();
        self.selected = 0;
        self.scroll.set_offset(point(px(0.), px(0.)));
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.results.get(self.selected).cloned() else {
            return;
        };
        window.close_dialog(cx);
        let _ = self.app.update(cx, |app, cx| match item {
            ResultItem::Session(hit) => {
                if let Some((workspace, _)) = app
                    .session_control
                    .locate(&app.store.workspaces, hit.session)
                {
                    app.switch(workspace, Some(hit.session), window, cx);
                }
            }
            ResultItem::File { key, .. } => app.open_workspace_file(key, true, window, cx),
        });
    }

    fn handle_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        }) {
            return;
        }
        match key {
            "up" | "down" if !self.results.is_empty() => {
                let len = self.results.len();
                self.selected = if key == "up" {
                    (self.selected + len - 1) % len
                } else {
                    (self.selected + 1) % len
                };
                self.scroll.scroll_to_item(self.selected);
                cx.notify();
            }
            "enter" => self.open_selected(window, cx),
            "escape" => window.close_dialog(cx),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Render for WorkspaceSearch {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.input.read(cx).value().to_string();
        let scanning = self
            .scanning
            .iter()
            .any(|root| self.scope.as_ref().is_none_or(|scope| scope == root));
        let loading = self.searching || scanning;
        let result_rows = if loading && self.results.is_empty() {
            5
        } else {
            self.results.len().min(8)
        };
        let content_height = (result_rows as f32 * 40.)
            .max((self.workspaces.len() + 1).min(10) as f32 * 28. + 40.)
            .clamp(120., 336.);
        let height = (content_height + 104.)
            .min((f32::from(window.viewport_size().height) - 160.).max(224.));
        let mut categories = h_flex().pb_2().gap_1().flex_shrink_0();
        for (index, category) in [Category::All, Category::Sessions, Category::Files]
            .into_iter()
            .enumerate()
        {
            let count = match category {
                Category::All => self.counts.0 + self.counts.1,
                Category::Sessions => self.counts.0,
                Category::Files => self.counts.1,
            };
            categories = categories.child(
                Button::new(("workspace-search-category", index))
                    .debug_selector(move || format!("workspace-search-category-{index}"))
                    .small()
                    .ghost()
                    .h(px(28.))
                    .px_2()
                    .selected(category == self.category)
                    .label(format!("{}  {count}", category.label()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.category = category;
                        this.filter_category();
                        cx.notify();
                        this.input.read(cx).focus_handle(cx).focus(window, cx);
                    })),
            );
        }
        let mut scopes = v_flex()
            .w(px(168.))
            .h_full()
            .flex_shrink_0()
            .p_2()
            .gap_0()
            .bg(rgb(palette().sidebar_bg))
            .border_r_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static("search.scope")),
            )
            .child(
                Button::new("workspace-search-all-scopes")
                    .debug_selector(|| "workspace-search-all-scopes".into())
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .accessibility_label(conduit::i18n::text_static("search.all_workspaces"))
                    .child(
                        div()
                            .w_full()
                            .text_left()
                            .truncate()
                            .child(conduit::i18n::text_static("search.all_workspaces")),
                    )
                    .selected(self.scope.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.scope = None;
                        this.refresh(cx);
                        this.input.read(cx).focus_handle(cx).focus(window, cx);
                    })),
            );
        let mut scope_list = v_flex()
            .id("workspace-search-scopes")
            .debug_selector(|| "workspace-search-scopes".into())
            .flex_1()
            .min_h_0()
            .gap_0()
            .overflow_y_scroll();
        for (index, (root, name)) in self.workspaces.iter().enumerate() {
            let root = root.clone();
            scope_list = scope_list.child(
                Button::new(("workspace-search-scope", index))
                    .debug_selector(move || format!("workspace-search-scope-{index}"))
                    .ghost()
                    .small()
                    .w_full()
                    .h(px(28.))
                    .px_2()
                    .accessibility_label(name.clone())
                    .child(div().w_full().text_left().truncate().child(name.clone()))
                    .tooltip(display_path(&root))
                    .selected(self.scope.as_ref() == Some(&root))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.scope = Some(root.clone());
                        this.refresh(cx);
                        this.input.read(cx).focus_handle(cx).focus(window, cx);
                    })),
            );
        }
        scopes = scopes.child(scope_list);
        let mut results = v_flex()
            .id("workspace-search-results")
            .debug_selector(|| "workspace-search-results".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll);
        for (index, item) in self.results.iter().enumerate() {
            let subtitle = match item {
                ResultItem::Session(hit) => {
                    let context = if hit.archived {
                        format!(
                            "{} · {}",
                            hit.workspace_name,
                            conduit::i18n::text_static("search.archived")
                        )
                    } else {
                        hit.workspace_name.clone()
                    };
                    if hit.snippet.is_empty() {
                        context
                    } else {
                        format!("{context} · {}", hit.snippet.replace(['\r', '\n'], " "))
                    }
                }
                ResultItem::File {
                    key,
                    workspace_name,
                } => format!("{workspace_name} · {}", display_path(&key.relative)),
            };
            results = results.child(
                h_flex()
                    .id(("workspace-search-result", index))
                    .debug_selector(move || format!("workspace-search-result-{index}"))
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .rounded(px(6.))
                    .when(index == self.selected, |row| {
                        row.bg(rgb(palette().surface_hover))
                    })
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.selected = index;
                        this.open_selected(window, cx);
                    }))
                    .child(
                        Icon::new(item.icon())
                            .small()
                            .text_color(rgb(palette().muted)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0()
                            .child(div().line_height(ui_text_size(18.)).truncate().child(
                                highlighted_text(&item.title(), &query, 13., palette().text),
                            ))
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .line_height(ui_text_size(14.))
                                    .text_color(rgb(palette().muted))
                                    .truncate()
                                    .child(subtitle),
                            ),
                    ),
            );
        }
        if loading && self.results.is_empty() {
            results = results.children((0usize..5).map(|index| {
                h_flex()
                    .id(("workspace-search-skeleton", index))
                    .debug_selector(move || format!("workspace-search-skeleton-{index}"))
                    .h(px(40.))
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .child(
                        Skeleton::new()
                            .size(px(14.))
                            .rounded(px(4.))
                            .flex_shrink_0(),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_2()
                            .child(
                                Skeleton::new()
                                    .w(px(180. - index as f32 * 12.))
                                    .h(px(12.))
                                    .rounded(px(3.)),
                            )
                            .child(
                                Skeleton::new()
                                    .w(px(96. + index as f32 * 8.))
                                    .h(px(10.))
                                    .rounded(px(3.))
                                    .secondary(),
                            ),
                    )
            }));
        } else if self.results.is_empty() {
            results = results.child(
                div()
                    .px_2()
                    .py_1()
                    .text_color(rgb(palette().muted))
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static(if self.searching || scanning {
                        "file_picker.searching"
                    } else {
                        "search.no_results"
                    })),
            );
        }
        let mut center = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_2()
            .gap_0()
            .child(results);
        if self.errors > 0
            || self.indexes.iter().any(|(root, index)| {
                index.incomplete && self.scope.as_ref().is_none_or(|scope| scope == root)
            })
        {
            center = center.child(
                div()
                    .px_2()
                    .py_1()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().warning_text))
                    .child(conduit::i18n::text_static("search.partial_results")),
            );
        }
        let body = h_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .border_1()
            .border_color(rgb(palette().border))
            .rounded(px(8.))
            .overflow_hidden()
            .child(scopes)
            .child(center);
        v_flex()
            .id("workspace-search")
            .debug_selector(|| "workspace-search".into())
            .w_full()
            .h(px(height))
            .min_w_0()
            .key_context("WorkspaceSearch")
            .on_action(
                cx.listener(|this, _: &PreviousResult, window, cx| {
                    this.handle_key("up", window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &NextResult, window, cx| this.handle_key("down", window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenResult, window, cx| {
                    this.handle_key("enter", window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &CloseSearch, window, cx| {
                this.handle_key("escape", window, cx)
            }))
            .child(
                div().pb_2().flex_shrink_0().child(
                    Input::new(&self.input)
                        .h(px(32.))
                        .prefix(Icon::new(FullIconName::Search).small())
                        .focus_bordered(false),
                ),
            )
            .child(categories)
            .child(body)
            .child(
                h_flex()
                    .h(px(24.))
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "{}{}",
                        self.results.len(),
                        if self.searching || scanning {
                            "…"
                        } else {
                            ""
                        }
                    ))
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .children(Kbd::binding_for_action(
                                        &PreviousResult,
                                        Some("WorkspaceSearch"),
                                        window,
                                    ))
                                    .children(Kbd::binding_for_action(
                                        &NextResult,
                                        Some("WorkspaceSearch"),
                                        window,
                                    ))
                                    .child(conduit::i18n::text_static("search.key_navigate")),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .children(Kbd::binding_for_action(
                                        &OpenResult,
                                        Some("WorkspaceSearch"),
                                        window,
                                    ))
                                    .child(conduit::i18n::text_static("search.key_open")),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1()
                                    .children(Kbd::binding_for_action(
                                        &CloseSearch,
                                        Some("WorkspaceSearch"),
                                        window,
                                    ))
                                    .child(conduit::i18n::text_static("search.key_close")),
                            ),
                    ),
            )
    }
}

impl ConduitApp {
    pub(in crate::app) fn open_workspace_search(
        &mut self,
        files: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || window.has_active_dialog(cx) || self.composer_is_composing(window, cx) {
            return;
        }
        let app = cx.weak_entity();
        let workspaces = self
            .store
            .workspaces
            .iter()
            .map(|workspace| (workspace.path.clone(), workspace.name()))
            .collect();
        let scope = self.selected_workspace_path();
        let search = cx.new(|cx| {
            WorkspaceSearch::new(
                app,
                if files {
                    Category::Files
                } else {
                    Category::All
                },
                workspaces,
                scope,
                window,
                cx,
            )
        });
        let initial_focus = std::cell::Cell::new(true);
        window.open_dialog(cx, move |dialog, window, cx| {
            if initial_focus.replace(false) {
                search
                    .read(cx)
                    .input
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }
            let width = (f32::from(window.viewport_size().width) - 48.).clamp(240., 880.);
            dialog
                .title(conduit::i18n::text_static("search.everything"))
                .w(px(width))
                .p_3()
                .pb_1()
                .child(search.clone())
        });
    }
}
