//! Repository request navigation, with independent state for each workspace.
use super::*;
use crate::github::{forge::Forge, list::Entry};

pub(in crate::app) struct RequestSidebar {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    input: Option<Entity<InputState>>,
    entries: Vec<Entry>,
    forge: Option<Forge>,
    filters: [Option<String>; 5],
    filters_expanded: bool,
    selected: Option<(Forge, u64)>,
    loading: bool,
    error: Option<String>,
    scroll: UniformListScrollHandle,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
    _input_subscription: Option<Subscription>,
}

impl RequestSidebar {
    pub(in crate::app) fn new(
        host: WeakEntity<ConduitApp>,
        workspace: PathBuf,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut sidebar = Self {
            host,
            workspace,
            input: None,
            entries: Vec::new(),
            forge: None,
            filters: Default::default(),
            filters_expanded: false,
            selected: None,
            loading: false,
            error: None,
            scroll: Default::default(),
            cancel: Arc::new(AtomicBool::new(false)),
            _task: Task::ready(()),
            _input_subscription: None,
        };
        sidebar.reload(None, cx);
        sidebar
    }

    fn reload(&mut self, provider: Option<Option<Forge>>, cx: &mut Context<Self>) {
        if provider.is_some()
            && self.host.upgrade().is_none_or(|host| {
                let app = host.read(cx);
                !app.writable
                    || app.closing
                    || !app
                        .store
                        .workspaces
                        .iter()
                        .any(|w| w.path == self.workspace)
            })
        {
            return;
        }
        self.cancel.store(true, Ordering::Release);
        self.cancel = Arc::new(AtomicBool::new(false));
        self.loading = true;
        self.error = None;
        if provider.is_some() {
            self.entries.clear();
            self.filters = Default::default();
            self.selected = None;
        }
        let cancel = self.cancel.clone();
        let workspace = self.workspace.clone();
        self._task = cx.spawn(async move |this, cx| {
            let ticket = cancel.clone();
            let (forge, result) = cx
                .background_executor()
                .spawn(async move {
                    let target = (|| {
                        if let Some(provider) = provider {
                            crate::github::forge::select(&workspace, provider, &ticket)?;
                        }
                        crate::github::forge::target(&workspace, &ticket)
                    })();
                    match target {
                        Ok(target) => (
                            Some(target.forge),
                            crate::github::list::load(&workspace, &ticket),
                        ),
                        Err(error) => (None, Err(error)),
                    }
                })
                .await;
            if cancel.load(Ordering::Acquire) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                this.forge = forge;
                match result {
                    Ok(entries) => this.entries = entries,
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                if let Some(host) = this.host.upgrade() {
                    host.update(cx, |_, cx| cx.notify());
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    pub(in crate::app) fn tab_title(&self) -> &'static str {
        match self.forge {
            Some(Forge::Gitlab) => "MR",
            _ => "PR",
        }
    }

    pub(in crate::app) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(in crate::app) fn refresh(&mut self, cx: &mut Context<Self>) {
        self.reload(None, cx);
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |_, cx| cx.notify());
        }
    }

    fn values(entry: &Entry, filter: usize) -> Vec<&str> {
        match filter {
            0 => vec![entry.state.as_str()],
            1 => vec![entry.author.as_str()],
            2 => entry.assignees.iter().map(String::as_str).collect(),
            3 => entry.reviewers.iter().map(String::as_str).collect(),
            _ => entry.labels.iter().map(String::as_str).collect(),
        }
    }

    fn matches(&self, entry: &Entry, query: &str) -> bool {
        self.filters.iter().enumerate().all(|(index, value)| {
            value
                .as_ref()
                .is_none_or(|value| Self::values(entry, index).contains(&value.as_str()))
        }) && (query.is_empty()
            || format!(
                "{} {} {} {}",
                entry.number, entry.title, entry.author, entry.branch
            )
            .to_lowercase()
            .contains(query))
    }

    fn open(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let entry = &self.entries[index];
        let requested = (entry.forge, entry.number);
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |app, cx| {
                if app.selected_workspace_path().as_ref() == Some(&self.workspace) {
                    app.open_github_request(
                        Some((requested.0, requested.1, entry.url.clone())),
                        window,
                        cx,
                    );
                }
            });
            self.selected = Some(requested);
            cx.notify();
        }
    }

    fn row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let entry = &self.entries[index];
        let selected = self.selected == Some((entry.forge, entry.number));
        let color = match entry.state.as_str() {
            "OPEN" => palette().git_added_text,
            "CLOSED" => palette().muted,
            _ => palette().activity_running,
        };
        let date = chrono::DateTime::parse_from_rfc3339(&entry.created_at)
            .map(|date| date.with_timezone(&Local).format("%Y/%m/%d").to_string())
            .unwrap_or_default();
        let tooltip = format!("{}\n{}\n{}", entry.title, entry.branch, entry.url);
        Button::new(("request-row", index))
            .debug_selector(move || format!("request-row-{index}"))
            .ghost()
            .selected(selected)
            .w_full()
            .h(px(52.))
            .p_0()
            .rounded_none()
            .min_w_0()
            .cursor_pointer()
            .tooltip(tooltip)
            .accessibility_label(format!(
                "{} · {} · {} · {}",
                entry.forge.request_name(entry.number),
                entry.title,
                presentation::pr_state(&entry.state),
                entry.author
            ))
            .on_click(cx.listener(move |this, _, window, cx| this.open(index, window, cx)))
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .py_1()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().text))
                                    .child(entry.title.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .px_1()
                                    .rounded(px(2.))
                                    .bg(rgb(palette().surface_strong))
                                    .text_size(ui_text_size(10.))
                                    .text_color(rgb(color))
                                    .child(presentation::pr_state(&entry.state)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "{} · {} · {}",
                                        entry.forge.request_name(entry.number),
                                        date,
                                        entry.author
                                    )),
                            )
                            .children(
                                entry
                                    .assignees
                                    .iter()
                                    .chain(entry.reviewers.iter())
                                    .take(2)
                                    .map(|name| {
                                        div()
                                            .size(px(18.))
                                            .flex_shrink_0()
                                            .rounded_full()
                                            .bg(rgb(palette().surface_strong))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_size(ui_text_size(10.))
                                            .text_color(rgb(palette().activity_running))
                                            .child(
                                                name.chars()
                                                    .next()
                                                    .unwrap_or('?')
                                                    .to_uppercase()
                                                    .to_string(),
                                            )
                                    }),
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .flex_shrink_0()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(
                                        Icon::new(FullIconName::MessageSquare).with_size(px(12.)),
                                    )
                                    .child(entry.comments.to_string()),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Drop for RequestSidebar {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Render for RequestSidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.input.is_none() {
            let input =
                cx.new(|cx| InputState::new(window, cx).placeholder(label("github.list_search")));
            self._input_subscription =
                Some(cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                        cx.notify();
                    }
                }));
            self.input = Some(input);
        }
        let query = self
            .input
            .as_ref()
            .unwrap()
            .read(cx)
            .value()
            .trim()
            .trim_start_matches(['#', '!'])
            .to_lowercase();
        let indices: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| self.matches(entry, &query))
            .map(|(index, _)| index)
            .collect();
        let filtered = self.filters.iter().any(Option::is_some) || !query.is_empty();
        let mut filters = Vec::new();
        for (index, key) in [
            "github.filter_state",
            "github.filter_author",
            "github.filter_assignee",
            "github.filter_reviewer",
            "github.filter_label",
        ]
        .into_iter()
        .enumerate()
        {
            let mut options: Vec<String> = self
                .entries
                .iter()
                .flat_map(|entry| Self::values(entry, index).into_iter().map(str::to_owned))
                .collect();
            options.sort();
            options.dedup();
            let panel = cx.weak_entity();
            let active = self.filters[index].clone();
            filters.push(
                Button::new(("request-filter", index))
                    .debug_selector(move || format!("request-filter-{index}"))
                    .xsmall()
                    .compact()
                    .ghost()
                    .selected(active.is_some())
                    .label(active.map_or_else(
                        || label(key).to_owned(),
                        |value| {
                            if index == 0 {
                                presentation::pr_state(&value)
                            } else {
                                value
                            }
                        },
                    ))
                    .icon(IconName::ChevronDown)
                    .max_w(px(112.))
                    .flex_shrink_0()
                    .dropdown_menu(move |mut menu, _, _| {
                        for value in std::iter::once(None).chain(options.iter().cloned().map(Some))
                        {
                            let panel = panel.clone();
                            let title = value.as_ref().map_or_else(
                                || label("github.filter_all").to_owned(),
                                |value| {
                                    if index == 0 {
                                        presentation::pr_state(value)
                                    } else {
                                        value.clone()
                                    }
                                },
                            );
                            menu =
                                menu.item(PopupMenuItem::new(title).on_click(move |_, _, cx| {
                                    let _ = panel.update(cx, |this, cx| {
                                        this.filters[index] = value.clone();
                                        this.scroll.scroll_to_item(0, ScrollStrategy::Top);
                                        cx.notify();
                                    });
                                }));
                        }
                        menu
                    })
                    .into_any_element(),
            );
        }
        let state_filter = filters.remove(0);
        let advanced_count = self.filters[1..].iter().flatten().count();
        let mut content = v_flex()
            .id("request-sidebar")
            .debug_selector(|| "request-sidebar".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(
                v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .child(
                        h_flex()
                            .gap_1()
                            .min_w_0()
                            .items_center()
                            .child(
                                div().flex_1().min_w_0().child(
                                    Input::new(self.input.as_ref().unwrap())
                                        .h(px(28.))
                                        .prefix(Icon::new(FullIconName::Search).small())
                                        .focus_bordered(false),
                                ),
                            )
                            .child(state_filter)
                            .child(
                                Button::new("request-filters")
                                    .debug_selector(|| "request-filters".into())
                                    .small()
                                    .compact()
                                    .ghost()
                                    .icon(FullIconName::ListFilter)
                                    .selected(self.filters_expanded || advanced_count > 0)
                                    .when(advanced_count > 0, |button| {
                                        button.label(advanced_count.to_string())
                                    })
                                    .tooltip(label("github.filters"))
                                    .accessibility_label(label("github.filters"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.filters_expanded = !this.filters_expanded;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when(self.filters_expanded, |header| {
                        header.child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .flex_wrap()
                                .children(filters),
                        )
                    }),
            );
        if let Some(error) = &self.error {
            content = content.child(
                div()
                    .px_3()
                    .py_2()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if indices.is_empty() && self.error.is_none() {
            content = content.child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(label(if self.loading {
                        "github.loading"
                    } else if filtered {
                        "github.list_no_matches"
                    } else {
                        "github.list_empty"
                    })),
            );
        } else if !indices.is_empty() {
            content = content.child(
                uniform_list(
                    "request-list",
                    indices.len(),
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range.map(|index| this.row(indices[index], cx)).collect()
                    }),
                )
                .flex_1()
                .min_h_0()
                .track_scroll(&self.scroll),
            );
        }
        let panel = cx.weak_entity();
        content.child(
            h_flex()
                .mt_auto()
                .px_3()
                .py_1()
                .gap_2()
                .border_t_1()
                .border_color(rgb(palette().border))
                .flex_shrink_0()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(label("github.list_limit")),
                )
                .child(
                    Button::new("request-provider")
                        .xsmall()
                        .ghost()
                        .label(self.forge.map_or(label("forge.select"), Forge::name))
                        .disabled(
                            self.loading
                                || self.host.upgrade().is_none_or(|host| {
                                    !host.read(cx).writable || host.read(cx).closing
                                }),
                        )
                        .dropdown_menu(move |mut menu, _, _| {
                            for (name, provider) in [
                                (label("forge.auto"), None),
                                ("GitHub", Some(Forge::Github)),
                                ("GitLab", Some(Forge::Gitlab)),
                            ] {
                                let panel = panel.clone();
                                menu = menu.item(PopupMenuItem::new(name).on_click(
                                    move |_, _, cx| {
                                        let _ = panel
                                            .update(cx, |this, cx| this.reload(Some(provider), cx));
                                    },
                                ));
                            }
                            menu
                        }),
                ),
        )
    }
}

impl ConduitApp {
    pub(in crate::app) fn request_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(workspace) = self.selected_workspace_path() else {
            return div()
                .p_3()
                .child(label("files.git_changes_no_workspace_selected"))
                .into_any_element();
        };
        let host = cx.weak_entity();
        self.github_ui
            .sidebars
            .entry(workspace.clone())
            .or_insert_with(|| cx.new(|cx| RequestSidebar::new(host, workspace, cx)))
            .clone()
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{Entry, Forge, RequestSidebar};
    use crate::app::files;
    use gpui_kit::{AppContext as _, Task, TestAppContext};
    use std::{
        collections::HashSet,
        sync::{Arc, atomic::AtomicBool},
    };

    #[gpui_kit::test]
    fn request_tabs_preserve_commit_selection_draft_and_workspace_filters(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = crate::app::queue::tests::fixture(cx, "request-sidebar-state");
        let sidebar = cx.update(|window, cx| {
            let sidebar = cx.new(|_| RequestSidebar {
                host: app.downgrade(),
                workspace: root.clone(),
                input: None,
                entries: vec![Entry {
                    forge: Forge::Github,
                    number: 7,
                    title: "修复 CI 构建".into(),
                    url: "https://github.com/example/repo/pull/7".into(),
                    branch: "fix-ci".into(),
                    state: "OPEN".into(),
                    author: "alice".into(),
                    created_at: "2026-11-17T08:00:00Z".into(),
                    assignees: vec!["alice".into()],
                    reviewers: vec!["bob".into()],
                    labels: vec!["bug".into()],
                    comments: 2,
                }],
                forge: Some(Forge::Github),
                filters: Default::default(),
                filters_expanded: false,
                selected: None,
                loading: false,
                error: None,
                scroll: Default::default(),
                cancel: Arc::new(AtomicBool::new(false)),
                _task: Task::ready(()),
                _input_subscription: None,
            });
            app.update(cx, |app, cx| {
                app.startup = None;
                app.writable = false;
                app.ensure_commit_draft(window, cx);
                app.git.drafts[&root].input.update(cx, |input, cx| {
                    input.set_value("Keep commit draft", window, cx)
                });
                app.git
                    .selected_files
                    .insert(root.clone(), HashSet::from(["src/main.rs".into()]));
                app.github_ui.sidebars.insert(root.clone(), sidebar.clone());
                app.open_github(window, cx);
            });
            sidebar
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("request-row-0").is_some());
        cx.simulate_resize(gpui_kit::size(gpui_kit::px(850.), gpui_kit::px(500.)));
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let navigation = cx.debug_bounds("git-navigation").unwrap();
        let repository = cx.debug_bounds("git-repository-context").unwrap();
        let request_tab = cx.debug_bounds("context-pr").unwrap();
        let picker = cx.debug_bounds("git-workspace").unwrap();
        assert!(repository.top() >= request_tab.bottom());
        assert!(picker.left() >= navigation.left() && picker.right() <= navigation.right());
        assert!(cx.debug_bounds("request-filter-0").is_some());
        assert!(cx.debug_bounds("request-filter-1").is_none());
        let filters = cx.debug_bounds("request-filters").unwrap();
        assert!(filters.right() <= navigation.right());
        cx.simulate_mouse_down(
            filters.center(),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            filters.center(),
            gpui_kit::MouseButton::Left,
            gpui_kit::Modifiers::default(),
        );
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("request-filter-1").is_some());
        cx.update(|_, cx| {
            sidebar.update(cx, |sidebar, _| {
                sidebar.forge = Some(Forge::Gitlab);
                assert_eq!(sidebar.tab_title(), "MR");
                sidebar.forge = Some(Forge::Github);
                assert_eq!(sidebar.tab_title(), "PR");
            });
        });
        cx.update(|window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                sidebar.filters[0] = Some("MERGED".into());
                assert!(!sidebar.matches(&sidebar.entries[0], "ci"));
                sidebar.filters = [
                    Some("OPEN".into()),
                    Some("alice".into()),
                    Some("alice".into()),
                    Some("bob".into()),
                    Some("bug".into()),
                ];
                assert!(sidebar.matches(&sidebar.entries[0], "fix-ci"));
                sidebar
                    .input
                    .as_ref()
                    .unwrap()
                    .update(cx, |input, cx| input.set_value("fix-ci", window, cx));
            });
            app.update(cx, |app, cx| {
                app.github_ui.sidebar_active = false;
                assert_eq!(
                    app.git.drafts[&root].input.read(cx).value().as_str(),
                    "Keep commit draft"
                );
                assert!(app.git.selected_files[&root].contains("src/main.rs"));
                app.select_document(files::DocumentTab::Chat(2), window, cx);
                app.select_document(files::DocumentTab::Chat(1), window, cx);
                app.open_github(window, cx);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            assert_eq!(app.read(cx).github_ui.sidebars[&root], sidebar);
            assert_eq!(
                sidebar
                    .read(cx)
                    .input
                    .as_ref()
                    .unwrap()
                    .read(cx)
                    .value()
                    .as_str(),
                "fix-ci"
            );
            assert_eq!(sidebar.read(cx).filters[3].as_deref(), Some("bob"));
            app.update(cx, |app, _| app.forget_workspace_files(&root));
            assert!(!app.read(cx).github_ui.sidebars.contains_key(&root));
        });
    }
}
