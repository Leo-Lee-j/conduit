use super::*;

pub(crate) struct BranchTree {
    view: WeakEntity<GitLogView>,
    _subscription: Subscription,
}
impl BranchTree {
    pub(super) fn new(view: WeakEntity<GitLogView>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(
            &view.upgrade().expect("branch manager exists"),
            |_, _, cx| cx.notify(),
        );
        Self {
            view,
            _subscription: subscription,
        }
    }
}
impl Render for BranchTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.view
            .update(cx, |view, cx| view.render_tree(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl GitLogView {
    fn favorite_refs(&self, cx: &App) -> HashSet<String> {
        self.favorite_scope
            .as_ref()
            .and_then(|scope| {
                self.app
                    .upgrade()
                    .and_then(|app| app.read(cx).store.git.favorite_branches.get(scope).cloned())
            })
            .unwrap_or_default()
            .into_iter()
            .collect()
    }

    fn toggle_favorite(&self, reference: String, cx: &mut Context<Self>) {
        let (Some(app), Some(scope)) = (self.app.upgrade(), self.favorite_scope.clone()) else {
            return;
        };
        app.update(cx, |app, cx| {
            let favorites = app.store.git.favorite_branches.entry(scope).or_default();
            if favorites.contains(&reference) {
                favorites.retain(|r| r != &reference);
            } else {
                favorites.push(reference);
            }
            app.persist();
            cx.notify();
        });
        cx.notify();
    }

    fn group_header(
        &self,
        id: String,
        label: String,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let click_id = id.clone();
        h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .w_full()
            .h(px(28.))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .cursor_pointer()
            .text_size(px(11.))
            .text_color(rgb(palette().muted))
            .hover(|row| row.bg(rgb(palette().surface_hover)))
            .child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(12.)),
            )
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                match click_id.as_str() {
                    "git-branches-local-section" => this.interaction.update(cx, |s, cx| {
                        s.local_expanded = !s.local_expanded;
                        cx.notify();
                    }),
                    "git-branches-remote-section" => this.interaction.update(cx, |s, cx| {
                        s.remote_expanded = !s.remote_expanded;
                        cx.notify();
                    }),
                    _ => {
                        if !this.collapsed.remove(&click_id) {
                            this.collapsed.insert(click_id.clone());
                        }
                    }
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn branch_row(
        &self,
        index: usize,
        branch: &git::BranchInfo,
        favorites: &HashSet<String>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let current = self
            .repository
            .as_ref()
            .and_then(|r| r.context.current_ref.as_ref())
            == Some(&branch.full_ref);
        let selected = self.query.reference.as_ref() == Some(&branch.full_ref);
        let favorite = favorites.contains(&branch.full_ref);
        let group: SharedString = format!("git-branch-row-{index}").into();
        let reference = branch.full_ref.clone();
        let star_ref = reference.clone();
        let host = self.host();
        let bounds_host = host.clone();
        let bounds_ref = reference.clone();
        let button_host = host.clone();
        let button_info = branch.clone();
        let button_app = app.clone();
        let menu_info = branch.clone();
        let name = branch.name.clone();
        let caption = if branch.remote {
            self.repository
                .as_ref()
                .and_then(|repo| {
                    repo.remotes
                        .iter()
                        .filter_map(|r| name.strip_prefix(&format!("{r}/")))
                        .min_by_key(|s| s.len())
                })
                .unwrap_or(&name)
                .to_string()
        } else {
            name.clone()
        };
        let hint = format!(
            "{}{}{}",
            name,
            branch
                .upstream
                .as_ref()
                .map(|u| format!(" · 跟踪 {}", u.trim_start_matches("refs/remotes/")))
                .unwrap_or_default(),
            branch
                .occupied
                .as_ref()
                .map(|p| format!(" · {}", display_path(p)))
                .unwrap_or_default()
        );
        h_flex()
            .id(("git-branch-row", index))
            .debug_selector(move || format!("git-branch-row-{index}"))
            .group(group.clone())
            .h(px(30.))
            .flex_shrink_0()
            .w_full()
            .min_w_0()
            .pl(px(if branch.remote { 28. } else { 16. }))
            .pr_1()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .when(selected, |row| row.bg(rgb(palette().surface_hover)))
            .hover(|row| row.bg(rgb(palette().surface_hover)))
            .on_prepaint(move |bounds, _, cx| {
                bounds_host.record_branch_bounds(bounds_ref.clone(), bounds, cx)
            })
            .on_click(
                cx.listener(move |this, _, _, cx| this.select_ref(Some(reference.clone()), cx)),
            )
            .context_menu(move |menu, window, cx| {
                git_branches::branch_menu(
                    menu,
                    app.clone(),
                    menu_info.clone(),
                    host.clone(),
                    window,
                    cx,
                )
            })
            .child(
                Icon::new(if current {
                    FullIconName::Check
                } else {
                    FullIconName::GitBranch
                })
                .size(px(13.))
                .text_color(rgb(if current {
                    palette().git_added_text
                } else {
                    palette().muted
                })),
            )
            .child(
                div()
                    .id(("git-branch-name", index))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .when(current, |row| row.font_semibold())
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(hint.clone()).build(window, cx)
                    })
                    .child(caption),
            )
            .child(
                Button::new(("git-branch-favorite", index))
                    .debug_selector(move || format!("git-branch-favorite-{index}"))
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Star)
                    .selected(favorite)
                    .opacity(0.)
                    .group_hover(group.clone(), |style| style.opacity(1.))
                    .focus(|style| style.opacity(1.))
                    .tooltip(if favorite {
                        "取消收藏"
                    } else {
                        "收藏分支"
                    })
                    .accessibility_label(if favorite {
                        "取消收藏分支"
                    } else {
                        "收藏分支"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_favorite(star_ref.clone(), cx);
                    })),
            )
            .child(
                Button::new(("git-branch-actions", index))
                    .debug_selector(move || format!("git-branch-actions-{index}"))
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Ellipsis)
                    .opacity(0.)
                    .group_hover(group, |style| style.opacity(1.))
                    .focus(|style| style.opacity(1.))
                    .tooltip(format!("{name} 的分支操作"))
                    .accessibility_label(format!("{name} 的分支操作"))
                    .dropdown_menu(move |menu, window, cx| {
                        git_branches::branch_menu(
                            menu,
                            button_app.clone(),
                            button_info.clone(),
                            button_host.clone(),
                            window,
                            cx,
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_tree(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let host = self.host();
        let outside = host.clone();
        let favorites = self.favorite_refs(cx);
        let search = self.branch_search.read(cx).value().to_lowercase();
        let searching = !search.is_empty();
        let picker = self.app.upgrade().map(|app| {
            let path = self.workspace.clone().unwrap_or_default();
            let label = self
                .workspace
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "选择工作区".into());
            app.update(cx, |app, cx| {
                app.navigation_workspace_picker("branches-workspace", label, path, cx)
            })
        });
        let mut list = v_flex()
            .id("git-branches")
            .debug_selector(|| "git-branches".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll();
        let all_selected = self.query.reference.is_none();
        list = list.child(
            h_flex()
                .id("git-log-all-branches")
                .debug_selector(|| "git-log-all-branches".into())
                .h(px(30.))
                .flex_shrink_0()
                .px_3()
                .items_center()
                .gap_2()
                .cursor_pointer()
                .text_size(px(12.))
                .when(all_selected, |row| row.bg(rgb(palette().surface_hover)))
                .hover(|row| row.bg(rgb(palette().surface_hover)))
                .child(Icon::new(FullIconName::GitFork).size(px(13.)))
                .child("全部分支")
                .on_click(cx.listener(|this, _, _, cx| this.select_ref(None, cx))),
        );
        if let Some(repository) = &self.repository {
            let bounds_host = host.clone();
            let current_ref = repository.context.current_ref.clone();
            list = list.child(
                h_flex()
                    .id("git-log-head")
                    .debug_selector(|| "git-log-head".into())
                    .h(px(30.))
                    .flex_shrink_0()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .when(self.query.reference.as_deref() == Some("HEAD"), |row| {
                        row.bg(rgb(palette().surface_hover))
                    })
                    .hover(|row| row.bg(rgb(palette().surface_hover)))
                    .child(Icon::new(FullIconName::GitCommitHorizontal).size(px(13.)))
                    .child(div().min_w_0().truncate().child(format!(
                        "HEAD · {}{}",
                        repository.current,
                        if repository.detached {
                            "（分离）"
                        } else {
                            "（当前分支）"
                        }
                    )))
                    .on_prepaint(move |bounds, _, cx| {
                        bounds_host.record_branch_bounds("HEAD".into(), bounds, cx);
                        if let Some(reference) = &current_ref {
                            bounds_host.record_branch_bounds(reference.clone(), bounds, cx);
                        }
                    })
                    .on_click(
                        cx.listener(|this, _, _, cx| this.select_ref(Some("HEAD".into()), cx)),
                    ),
            );
            let mut branches: Vec<_> = repository
                .branch_info
                .iter()
                .enumerate()
                .filter(|(_, b)| b.name.to_lowercase().contains(&search))
                .map(|(i, b)| (i, b.clone()))
                .collect();
            branches.sort_by_key(|(_, b)| {
                (
                    !favorites.contains(&b.full_ref),
                    repository.context.current_ref.as_ref() != Some(&b.full_ref),
                    b.name.clone(),
                )
            });
            let local_expanded = self.interaction.read(cx).local_expanded || searching;
            let remote_expanded = self.interaction.read(cx).remote_expanded || searching;
            list = list.child(self.group_header(
                "git-branches-local-section".into(),
                "Local · 本地分支".into(),
                local_expanded,
                cx,
            ));
            if local_expanded {
                for (index, branch) in branches.iter().filter(|(_, b)| !b.remote) {
                    list = list.child(self.branch_row(*index, branch, &favorites, cx));
                }
            }
            list = list.child(self.group_header(
                "git-branches-remote-section".into(),
                "Remote · 远程分支".into(),
                remote_expanded,
                cx,
            ));
            if remote_expanded {
                let mut remotes: std::collections::BTreeMap<String, Vec<(usize, git::BranchInfo)>> =
                    Default::default();
                for (index, branch) in branches.iter().filter(|(_, b)| b.remote) {
                    let remote = repository
                        .remotes
                        .iter()
                        .filter(|r| branch.name.starts_with(&format!("{r}/")))
                        .max_by_key(|r| r.len())
                        .cloned()
                        .unwrap_or_else(|| {
                            branch.name.split('/').next().unwrap_or("remote").into()
                        });
                    remotes
                        .entry(remote)
                        .or_default()
                        .push((*index, branch.clone()));
                }
                for (remote, branches) in remotes {
                    let id = format!("git-remote-{remote}");
                    let expanded = !self.collapsed.contains(&id) || searching;
                    list = list.child(self.group_header(id, format!("  {remote}"), expanded, cx));
                    if expanded {
                        for (index, branch) in branches {
                            list = list.child(self.branch_row(index, &branch, &favorites, cx));
                        }
                    }
                }
            }
            if repository.truncated {
                list = list.child(div().p_2().text_size(px(11.)).child("分支列表已达显示上限"));
            }
        } else {
            list = list.child(
                div()
                    .p_3()
                    .text_size(px(12.))
                    .text_color(rgb(palette().muted))
                    .child(if self.workspace.is_none() {
                        "添加或选择工作区后查看分支"
                    } else {
                        "正在检测 Git 仓库…"
                    }),
            );
        }
        let expanded = !self.collapsed.contains("git-tags-section") || searching;
        list = list.child(self.group_header(
            "git-tags-section".into(),
            "Tags · 标签".into(),
            expanded,
            cx,
        ));
        if expanded && let Some(history) = &self.history {
            for (index, reference) in history
                .tags
                .iter()
                .enumerate()
                .filter(|(_, r)| r.to_lowercase().contains(&search))
            {
                let selected = self.query.reference.as_ref() == Some(reference);
                let value = reference.clone();
                list = list.child(
                    h_flex()
                        .id(("git-log-tag", index))
                        .debug_selector(move || format!("git-log-tag-{index}"))
                        .h(px(28.))
                        .flex_shrink_0()
                        .w_full()
                        .min_w_0()
                        .pl_5()
                        .pr_2()
                        .gap_2()
                        .items_center()
                        .cursor_pointer()
                        .when(selected, |row| row.bg(rgb(palette().surface_hover)))
                        .hover(|row| row.bg(rgb(palette().surface_hover)))
                        .child(Icon::new(FullIconName::Tag).size(px(12.)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .child(reference.trim_start_matches("refs/tags/").to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_ref(Some(value.clone()), cx)
                        })),
                );
            }
            if history.tags_truncated {
                list = list.child(
                    div()
                        .flex_shrink_0()
                        .p_2()
                        .text_size(px(11.))
                        .child("仅显示前 1000 个标签"),
                );
            }
        }
        let mut feedback_panel = v_flex()
            .id("git-branch-status")
            .max_h(px(110.))
            .overflow_y_scroll()
            .flex_shrink_0();
        if let Some(app) = self.app.upgrade() {
            let app = app.read(cx);
            if let Some(feedback) = self
                .workspace
                .as_ref()
                .and_then(|p| app.git.feedback.get(p))
            {
                feedback_panel = feedback_panel.child(
                    div()
                        .id("git-branch-feedback")
                        .p_2()
                        .text_size(px(11.))
                        .text_color(rgb(if feedback.error {
                            palette().danger_text
                        } else {
                            palette().muted
                        }))
                        .child(feedback.text.clone()),
                );
            }
            if let Some(message) = &app.message {
                feedback_panel = feedback_panel.child(
                    div()
                        .p_2()
                        .text_size(px(11.))
                        .text_color(rgb(palette().danger_text))
                        .child(message.clone()),
                );
            }
        }
        let integration = self
            .app
            .upgrade()
            .and_then(|app| app.read(cx).git_integration_banner(app.clone()));
        sidebar::navigation_pane()
            .min_h_0()
            .id("git-branch-navigation")
            .debug_selector(|| "git-branch-navigation".into())
            .on_mouse_down_out(move |event, window, cx| {
                outside.dismiss_unless_menu_open(event.position, window, cx)
            })
            .child(
                sidebar::navigation_header()
                    .debug_selector(|| "git-branch-header".into())
                    .px_2()
                    .items_center()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(div().text_size(px(12.)).font_semibold().child("分支"))
                    .child(div().flex_1())
                    .children(picker),
            )
            .child(
                div()
                    .p_2()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .debug_selector(|| "git-branch-search".into())
                    .child(
                        Input::new(&self.branch_search)
                            .h(px(30.))
                            .focus_bordered(false),
                    ),
            )
            .children(integration)
            .child(list)
            .child(feedback_panel)
            .children(host.side_form(cx))
            .into_any_element()
    }
}
