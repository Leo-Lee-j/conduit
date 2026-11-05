//! Context menus, branch forms and persistent integration recovery controls.
use super::*;
use gpui_kit::component::menu::PopupMenu;

pub(super) struct BranchMenuInteraction {
    menu: Option<WeakEntity<PopupMenu>>,
    form: Option<Entity<BranchForm>>,
    form_ref: Option<String>,
    form_bounds: Option<Bounds<Pixels>>,
    branch_bounds: HashMap<String, Bounds<Pixels>>,
    pub(super) local_expanded: bool,
    pub(super) remote_expanded: bool,
}

impl Default for BranchMenuInteraction {
    fn default() -> Self {
        Self {
            menu: None,
            form: None,
            form_ref: None,
            form_bounds: None,
            branch_bounds: HashMap::new(),
            local_expanded: true,
            remote_expanded: false,
        }
    }
}

#[derive(Clone)]
pub(super) struct BranchMenuHost {
    pub(super) workspace: PathBuf,
    pub(super) interaction: Entity<BranchMenuInteraction>,
    pub(super) popover: Option<WeakEntity<gpui_kit::component::popover::PopoverState>>,
}

impl BranchMenuHost {
    pub(super) fn dismiss_unless_menu_open(
        &self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self
            .interaction
            .read(cx)
            .form_bounds
            .is_some_and(|bounds| bounds.contains(&position))
        {
            return;
        }
        if self
            .interaction
            .read(cx)
            .menu
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .is_none()
        {
            self.interaction.update(cx, |state, cx| {
                state.form = None;
                state.form_bounds = None;
                cx.notify();
            });
            if let Some(popover) = self.popover.as_ref().and_then(WeakEntity::upgrade) {
                popover.update(cx, |state, cx| state.dismiss(window, cx));
            }
            window.refresh();
        }
    }

    pub(super) fn record_branch_bounds(
        &self,
        full_ref: String,
        bounds: Bounds<Pixels>,
        cx: &mut App,
    ) {
        self.interaction.update(cx, |state, _| {
            state.branch_bounds.insert(full_ref, bounds);
        });
    }

    pub(super) fn side_form(&self, cx: &App) -> Option<AnyElement> {
        let state = self.interaction.read(cx);
        let form = state.form.clone()?;
        let bounds = *state.branch_bounds.get(state.form_ref.as_ref()?)?;
        Some(
            deferred(
                gpui_kit::base::Positioner::side(bounds)
                    .placement(gpui_kit::base::Placement::Right)
                    .align(gpui_kit::base::Align::Start)
                    .offset(px(8.))
                    .margin(px(8.))
                    .occlude()
                    .child(form),
            )
            .with_priority(gpui_kit::base::POPUP_PRIORITY + 2)
            .into_any_element(),
        )
    }

    pub(super) fn open_simple_form(
        &self,
        app: Entity<ConduitApp>,
        repository: git::Repository,
        request: git::BranchRequest,
        kind: FormKind,
        window: &mut Window,
        cx: &mut Context<ConduitApp>,
    ) {
        let full_ref = request
            .selected
            .as_ref()
            .map(|b| b.full_ref.clone())
            .unwrap_or_else(|| "HEAD".into());
        let (_, form) = make_form(
            app,
            self.workspace.clone(),
            repository,
            request,
            kind,
            window,
            cx,
        );
        form.update(cx, |form, _| {
            form.popup_host = Some((self.interaction.downgrade(), self.popover.clone()));
        });
        self.interaction.update(cx, |state, cx| {
            state.form = Some(form.clone());
            state.form_ref = Some(full_ref);
            state.form_bounds = None;
            cx.notify();
        });
        window.refresh();
    }

    fn attach(&self, window: &mut Window, cx: &mut Context<PopupMenu>) {
        let menu = cx.entity().downgrade();
        self.interaction.update(cx, |state, _| {
            state.menu = Some(menu.clone());
            state.form = None;
            state.form_bounds = None;
        });
        let host = self.clone();
        window
            .subscribe(&cx.entity(), cx, move |_, _: &DismissEvent, window, cx| {
                let dismiss_parent = host.interaction.update(cx, |state, _| {
                    if state.menu.as_ref() == Some(&menu) {
                        state.menu = None;
                        return state.form.is_none();
                    }
                    false
                });
                if dismiss_parent
                    && let Some(popover) = host.popover.as_ref().and_then(WeakEntity::upgrade)
                {
                    popover.update(cx, |state, cx| state.dismiss(window, cx));
                }
                window.refresh();
            })
            .detach();
    }
}

#[derive(Clone)]
pub(super) enum FormKind {
    Create,
    Rename,
    Push,
    Worktree,
    Update(bool),
    Confirm(git::BranchAction),
}

#[derive(Clone)]
enum MenuAction {
    Run(git::BranchAction),
    Form(FormKind),
    Compare(git::ComparisonKind),
}

pub(super) fn branch_name_tag(name: String, max_width: f32) -> AnyElement {
    branch_tag(name, max_width, false)
}

pub(super) fn branch_tag(name: String, max_width: f32, current: bool) -> AnyElement {
    branch_tag_element(name, px(max_width), current)
}

fn branch_tag_element(name: String, max_width: Pixels, current: bool) -> AnyElement {
    let tooltip = name.clone();
    let tag = h_flex()
        .id(SharedString::from(format!("git-branch-tag-{name}")))
        .min_w_0()
        .max_w(max_width)
        .items_center()
        .gap(px(4.))
        .px(px(7.))
        .py(px(2.))
        .rounded(px(5.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface_strong))
        .when(current, |tag| {
            tag.bg(linear_gradient(
                90.,
                linear_color_stop(rgb(0xb9ddff), 0.),
                linear_color_stop(rgb(0xffffff), 1.),
            ))
            .border_color(rgb(0xb9d8f4))
        })
        .text_size(px(11.))
        .text_color(rgb(if current { 0x234764 } else { palette().text }))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .child(
            Icon::new(FullIconName::GitBranch)
                .size(px(12.))
                .flex_shrink_0()
                .text_color(rgb(if current { 0x467397 } else { palette().muted })),
        )
        .child(div().min_w_0().truncate().child(name));
    tag.into_any_element()
}

/// Keep the full accessible action name while rendering branch references as tags.
fn branch_action_item(label: String, names: Vec<(String, bool)>) -> PopupMenuItem {
    use gpui_kit::base::TestSupportExt as _;
    PopupMenuItem::element(move |_, _| {
        let mut row = h_flex()
            .id("git-branch-action-label")
            .role(Role::MenuItem)
            .aria_label(label.clone())
            .test_support()
            .min_w_0()
            .gap(px(4.));
        let mut rest = label.as_str();
        while let Some((offset, name, current)) = names
            .iter()
            .filter_map(|(name, current)| {
                rest.find(&format!("'{name}'"))
                    .map(|offset| (offset, name, *current))
            })
            .min_by_key(|(offset, _, _)| *offset)
        {
            if offset > 0 {
                row = row.child(rest[..offset].trim().to_string());
            }
            row = row.child(branch_tag(name.clone(), 170., current));
            rest = &rest[offset + name.len() + 2..];
        }
        if !rest.is_empty() {
            row = row.child(rest.trim().to_string());
        }
        row
    })
}

pub(super) fn branch_menu(
    mut menu: PopupMenu,
    app: Entity<ConduitApp>,
    selected: git::BranchInfo,
    host: BranchMenuHost,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    host.attach(window, cx);
    let workspace = host.workspace.clone();
    let state = app.read(cx);
    let Some(repository) = state
        .git
        .repository
        .clone()
        .filter(|_| state.selected_workspace_path().as_ref() == Some(&workspace))
    else {
        return menu;
    };
    let blocked = state.branch_operation_blocked(&workspace).is_some();
    let in_progress = repository.integration.is_some();
    let current = repository.context.current_ref.as_ref() == Some(&selected.full_ref);
    let occupied_elsewhere = selected
        .occupied
        .as_ref()
        .is_some_and(|path| fs_canonical(path) != fs_canonical(&git::worktree_root(&workspace)));
    let local = !selected.remote;
    let can_integrate =
        repository.context.current_ref.is_some() && repository.context.head.is_some() && !current;
    menu = menu
        .min_w(px(260.))
        .max_w(px((f32::from(window.viewport_size().width) - 48.).min(620.)))
        .max_h(px(
            (f32::from(window.viewport_size().height) - 48.).max(180.)
        ))
        .scrollable(true);
    let mut entries = Vec::new();
    if !current {
        entries.push((
            "Checkout".into(),
            MenuAction::Run(git::BranchAction::Checkout),
            occupied_elsewhere,
        ));
    }
    entries.push((
        format!("New Branch from '{}'", selected.name),
        MenuAction::Form(FormKind::Create),
        false,
    ));
    if can_integrate && !occupied_elsewhere {
        entries.push((
            format!("Checkout and Rebase onto '{}'", repository.current),
            MenuAction::Form(FormKind::Confirm(git::BranchAction::CheckoutRebase)),
            false,
        ));
    }
    if !current && !occupied_elsewhere {
        entries.push((
            "Checkout and Update".into(),
            MenuAction::Form(FormKind::Update(true)),
            false,
        ));
    }
    if repository.context.head.is_some() {
        entries.push((
            format!("Compare with '{}'", repository.current),
            MenuAction::Compare(git::ComparisonKind::Branches),
            current,
        ));
        entries.push((
            "Show Diff with Working Tree".into(),
            MenuAction::Compare(git::ComparisonKind::WorkingTree),
            false,
        ));
    }
    if can_integrate {
        entries.push((
            format!("Rebase '{}' onto '{}'", repository.current, selected.name),
            MenuAction::Form(FormKind::Confirm(git::BranchAction::Rebase)),
            false,
        ));
        entries.push((
            format!("Merge '{}' into '{}'", selected.name, repository.current),
            MenuAction::Form(FormKind::Confirm(git::BranchAction::Merge)),
            false,
        ));
    }
    entries.push((
        format!("New Worktree from '{}'", selected.name),
        MenuAction::Form(FormKind::Worktree),
        false,
    ));
    if local {
        entries.push((
            "Update".into(),
            MenuAction::Form(FormKind::Update(false)),
            selected.upstream.is_none() || occupied_elsewhere,
        ));
        entries.push((
            "Push".into(),
            MenuAction::Form(FormKind::Push),
            repository.remotes.is_empty() && selected.upstream_remote.as_deref() != Some("."),
        ));
    }
    for (label, action, disabled) in entries {
        let form_host = host.clone();
        let view = app.clone();
        let branch = selected.clone();
        let path = workspace.clone();
        let snapshot = repository.clone();
        let read_only = matches!(action, MenuAction::Compare(_));
        menu = menu.item(
            branch_action_item(
                label,
                vec![
                    (selected.name.clone(), current),
                    (repository.current.clone(), true),
                ],
            )
            .disabled(disabled || (!read_only && (blocked || in_progress)))
            .on_click(move |_, window, cx| {
                view.update(cx, |this, cx| {
                    let request = git::BranchRequest {
                        expected: snapshot.context.clone(),
                        selected: Some(branch.clone()),
                        action: git::BranchAction::Checkout,
                    };
                    match &action {
                        MenuAction::Run(action) => {
                            this.start_branch_operation(
                                path.clone(),
                                git::BranchRequest {
                                    action: action.clone(),
                                    ..request
                                },
                                cx,
                            );
                        }
                        MenuAction::Form(FormKind::Create) => form_host.open_simple_form(
                            cx.entity(),
                            snapshot.clone(),
                            request,
                            FormKind::Create,
                            window,
                            cx,
                        ),
                        MenuAction::Form(kind) => open_form(
                            cx.entity(),
                            path.clone(),
                            snapshot.clone(),
                            request,
                            kind.clone(),
                            window,
                            cx,
                        ),
                        MenuAction::Compare(kind) => this.open_git_comparison(
                            path.clone(),
                            git::ComparisonRequest {
                                selected: branch.clone(),
                                expected: snapshot.context.clone(),
                                kind: *kind,
                            },
                            window,
                            cx,
                        ),
                    }
                });
            }),
        );
    }
    if local {
        let upstream_app = app.clone();
        let branch = selected.clone();
        let path = workspace.clone();
        let snapshot = repository.clone();
        let upstream_menu = PopupMenu::build(window, cx, move |menu, window, _| {
            let mut menu =
                menu.scrollable(true).max_h(px(
                    (f32::from(window.viewport_size().height) - 48.).clamp(120., 320.)
                ));
            for target in std::iter::once(None).chain(
                snapshot
                    .branch_info
                    .iter()
                    .filter(|b| b.remote)
                    .map(|b| Some(b.full_ref.clone())),
            ) {
                let label = target
                    .as_deref()
                    .and_then(|s| s.strip_prefix("refs/remotes/"))
                    .unwrap_or("取消跟踪")
                    .to_string();
                let view = upstream_app.clone();
                let path = path.clone();
                let request = git::BranchRequest {
                    expected: snapshot.context.clone(),
                    selected: Some(branch.clone()),
                    action: git::BranchAction::Track(target.clone()),
                };
                menu = menu.item(
                    if target.is_some() {
                        let name = label.clone();
                        PopupMenuItem::element(move |_, _| {
                            use gpui_kit::base::TestSupportExt as _;
                            div()
                                .id("git-upstream-label")
                                .role(Role::MenuItem)
                                .aria_label(name.clone())
                                .test_support()
                                .child(branch_name_tag(name.clone(), 220.))
                        })
                    } else {
                        PopupMenuItem::new(label)
                    }
                    .checked(branch.upstream == target)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |this, cx| {
                            this.start_branch_operation(path.clone(), request.clone(), cx);
                        });
                    }),
                );
            }
            menu
        });
        menu = menu.item(PopupMenuItem::separator()).item(
            PopupMenuItem::submenu("Tracked Branch", upstream_menu)
                .disabled(blocked || in_progress || occupied_elsewhere),
        );
        for (label, kind, disabled) in [
            ("Rename", FormKind::Rename, occupied_elsewhere),
            (
                "Delete",
                FormKind::Confirm(git::BranchAction::Delete),
                current || selected.occupied.is_some(),
            ),
        ] {
            let form_host = host.clone();
            let view = app.clone();
            let path = workspace.clone();
            let snapshot = repository.clone();
            let branch = selected.clone();
            menu = menu.item(
                PopupMenuItem::new(label)
                    .disabled(blocked || in_progress || disabled)
                    .on_click(move |_, window, cx| {
                        view.update(cx, |_, cx| {
                            let request = git::BranchRequest {
                                expected: snapshot.context.clone(),
                                selected: Some(branch.clone()),
                                action: git::BranchAction::Checkout,
                            };
                            if matches!(kind, FormKind::Rename) {
                                form_host.open_simple_form(
                                    cx.entity(),
                                    snapshot.clone(),
                                    request,
                                    kind.clone(),
                                    window,
                                    cx,
                                );
                            } else {
                                open_form(
                                    cx.entity(),
                                    path.clone(),
                                    snapshot.clone(),
                                    request,
                                    kind.clone(),
                                    window,
                                    cx,
                                );
                            }
                        });
                    }),
            );
        }
    }
    if occupied_elsewhere {
        menu = menu.item(PopupMenuItem::label(format!(
            "已被 Worktree 占用：{}",
            selected.occupied.as_ref().unwrap().display()
        )));
    } else if blocked {
        menu = menu.item(PopupMenuItem::label("仓库有任务运行，暂不可执行写操作"));
    }
    if in_progress {
        menu = menu.item(PopupMenuItem::label("请先继续或中止进行中的 Git 操作"));
    }
    menu
}

fn fs_canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ConduitApp {
    fn branch_operation_blocked(&self, path: &Path) -> Option<&'static str> {
        if self.closing || !self.writable {
            Some("应用当前不可写")
        } else if self.git.operation.is_some() {
            Some("已有 Git 操作运行，请等待完成")
        } else if self.git.generation.is_some() {
            Some("请先完成或取消 Commit Message 生成")
        } else if self.worktree_has_runs(path) {
            Some("该仓库的工作区有 CLI 任务运行，请等待完成后再操作")
        } else {
            None
        }
    }

    pub(super) fn start_branch_operation(
        &mut self,
        path: PathBuf,
        request: git::BranchRequest,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some(reason) = self.branch_operation_blocked(&path) {
            self.message = Some(reason.into());
            cx.notify();
            return false;
        }
        if self.selected_workspace_path().as_ref() != Some(&path) {
            self.message = Some("工作区已切换，请重新打开分支菜单".into());
            cx.notify();
            return false;
        }
        if !self.branch_navigation_open() {
            self.show_git_navigation(cx);
        }
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&path),
            probe: git::OperationProbe::start(
                path.clone(),
                git::Operation::Branch(Box::new(request)),
            ),
            draft: String::new(),
        });
        self.git_feedback(path, "正在执行 Git 分支操作…", false, cx);
        true
    }

    pub(super) fn git_integration_banner(&self, app: Entity<Self>) -> Option<AnyElement> {
        let repository = self.git.repository.as_ref()?;
        let state = repository.integration?;
        let path = self.selected_workspace_path()?;
        let supported = matches!(
            state,
            git::IntegrationState::Merge | git::IntegrationState::Rebase
        );
        let blocked = self.branch_operation_blocked(&path).is_some();
        let mut banner = v_flex()
            .w_full()
            .p_2()
            .gap_2()
            .rounded(px(8.))
            .bg(rgb(palette().warning_bg))
            .text_size(px(12.))
            .text_color(rgb(palette().warning_text))
            .child(format!(
                "{} 进行中 · {} 个冲突文件",
                state.label(),
                repository.conflicts.len()
            ))
            .child(div().text_size(px(11.)).child(if supported {
                "在外部编辑器解决冲突并 git add 后，点击继续；也可以中止。"
            } else {
                "请先在终端完成或中止此操作，然后刷新。"
            }));
        for conflict in repository.conflicts.iter().take(20) {
            banner = banner.child(div().truncate().child(conflict.clone()));
        }
        if repository.conflicts.len() > 20 {
            banner = banner.child("更多冲突文件请在终端 git status 查看");
        }
        if supported {
            let mut buttons = h_flex().gap_2();
            for (label, action) in [
                ("继续", git::BranchAction::Continue(state)),
                ("中止…", git::BranchAction::Abort(state)),
            ] {
                let view = app.clone();
                let path = path.clone();
                let request = git::BranchRequest {
                    expected: repository.context.clone(),
                    selected: None,
                    action: action.clone(),
                };
                let snapshot = repository.clone();
                let continuing = matches!(action, git::BranchAction::Continue(_));
                buttons = buttons.child(
                    Button::new(SharedString::from(format!("git-recovery-{label}")))
                        .debug_selector(move || {
                            if continuing {
                                "git-recovery-continue".into()
                            } else {
                                "git-recovery-abort".into()
                            }
                        })
                        .label(label)
                        .small()
                        .ghost()
                        .disabled(
                            blocked
                                || (matches!(action, git::BranchAction::Continue(_))
                                    && !repository.conflicts.is_empty()),
                        )
                        .on_click(move |_, window, cx| {
                            view.update(cx, |this, cx| {
                                if matches!(action, git::BranchAction::Abort(_)) {
                                    open_form(
                                        cx.entity(),
                                        path.clone(),
                                        snapshot.clone(),
                                        request.clone(),
                                        FormKind::Confirm(action.clone()),
                                        window,
                                        cx,
                                    );
                                } else {
                                    this.start_branch_operation(path.clone(), request.clone(), cx);
                                }
                            });
                        }),
                );
            }
            banner = banner.child(buttons);
        }
        Some(banner.into_any_element())
    }
}

struct BranchForm {
    app: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    request: git::BranchRequest,
    kind: FormKind,
    title: &'static str,
    focus: FocusHandle,
    popup_host: Option<(
        WeakEntity<BranchMenuInteraction>,
        Option<WeakEntity<gpui_kit::component::popover::PopoverState>>,
    )>,
    fields: Vec<(String, Entity<InputState>)>,
    toggle: bool,
    helper: String,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

fn open_form(
    app: Entity<ConduitApp>,
    workspace: PathBuf,
    repository: git::Repository,
    request: git::BranchRequest,
    kind: FormKind,
    window: &mut Window,
    cx: &mut Context<ConduitApp>,
) {
    let (title, form) = make_form(app, workspace, repository, request, kind, window, cx);
    window.open_dialog(cx, move |dialog, window, _| {
        let viewport = window.viewport_size();
        let confirm = form.clone();
        dialog
            .title(title)
            .w(px((f32::from(viewport.width) - 48.).clamp(280., 560.)))
            .margin_top(px(56.))
            .on_ok(move |_, _, cx| confirm.update(cx, |form, cx| form.submit(cx)))
            .content({
                let form = form.clone();
                move |content, _, _| content.child(form.clone())
            })
    });
}

fn make_form(
    app: Entity<ConduitApp>,
    workspace: PathBuf,
    repository: git::Repository,
    request: git::BranchRequest,
    kind: FormKind,
    window: &mut Window,
    cx: &mut Context<ConduitApp>,
) -> (&'static str, Entity<BranchForm>) {
    let selected = request
        .selected
        .as_ref()
        .map(|b| b.name.as_str())
        .unwrap_or("HEAD");
    let (title, helper, fields, toggle) = match &kind {
        FormKind::Create => (
            "新建分支",
            format!("从 {selected} 创建分支。"),
            vec![("分支名", String::new())],
            true,
        ),
        FormKind::Rename => (
            "重命名分支",
            "仅重命名本地分支；远程分支名不变。".into(),
            vec![("分支名", selected.into())],
            false,
        ),
        FormKind::Push => {
            let branch = request.selected.as_ref().unwrap();
            let remote = branch
                .upstream_remote
                .clone()
                .or_else(|| {
                    repository
                        .remotes
                        .iter()
                        .find(|r| r.as_str() == "origin")
                        .cloned()
                })
                .or_else(|| repository.remotes.first().cloned())
                .unwrap_or_default();
            let destination = branch
                .upstream_ref
                .as_deref()
                .and_then(|r| r.strip_prefix("refs/heads/"))
                .unwrap_or(selected)
                .to_string();
            (
                "推送分支",
                format!(
                    "推送 {selected}。可用远程：{}",
                    repository.remotes.join(", ")
                ),
                vec![("远程仓库", remote), ("目标分支", destination)],
                branch.upstream.is_none(),
            )
        }
        FormKind::Worktree => {
            let branch_name = format!(
                "{}-worktree",
                selected.rsplit('/').next().unwrap_or("branch")
            );
            let parent = git::worktree_root(&workspace)
                .parent()
                .unwrap_or(&workspace)
                .to_path_buf();
            let path = parent.join(&branch_name);
            (
                "创建 Worktree",
                format!(
                    "从 {selected} 创建独立工作目录，成功后加入左侧工作区。新分支名留空时使用所选本地分支。"
                ),
                vec![
                    ("目标目录（绝对路径）", path.to_string_lossy().into_owned()),
                    ("新分支名（可选）", branch_name),
                ],
                false,
            )
        }
        FormKind::Update(checkout) => (
            if *checkout {
                "切换并更新分支"
            } else {
                "更新分支"
            },
            format!(
                "{}更新 {selected} 的上游。非当前分支只允许快进；分叉时请先切换。",
                if *checkout { "先切换，再" } else { "" }
            ),
            vec![],
            false,
        ),
        FormKind::Confirm(action) => {
            let (title, helper) = match action {
                git::BranchAction::Delete => (
                    "删除本地分支",
                    format!("删除 {selected}？普通删除会拒绝未合并分支；不会删除远程分支。"),
                ),
                git::BranchAction::Merge => (
                    "合并分支",
                    format!(
                        "将 {selected} 合并到 {}。发生冲突时，可在外部解决后继续或中止。",
                        repository.current
                    ),
                ),
                git::BranchAction::Rebase => (
                    "Rebase 当前分支",
                    format!(
                        "将 {} Rebase 到 {selected}。这会改写当前分支的提交历史。",
                        repository.current
                    ),
                ),
                git::BranchAction::CheckoutRebase => (
                    "切换并 Rebase",
                    format!(
                        "先切换到 {selected}，再将它 Rebase 到 {}。这会改写 {selected} 的提交历史。",
                        repository.current
                    ),
                ),
                git::BranchAction::Abort(state) => (
                    "中止 Git 操作",
                    format!(
                        "中止进行中的 {}？Git 将尝试恢复操作前的状态；冲突解决中的修改可能被撤销。",
                        state.label()
                    ),
                ),
                _ => ("Git 操作", "确认执行此操作？".into()),
            };
            (title, helper, vec![], false)
        }
    };
    let form = cx.new(|cx| {
        let fields: Vec<_> = fields
            .into_iter()
            .map(|(label, value)| {
                (
                    label.into(),
                    cx.new(|cx| InputState::new(window, cx).default_value(value)),
                )
            })
            .collect();
        let subscriptions = fields
            .iter()
            .map(|(_, input)| {
                cx.subscribe(input, |this: &mut BranchForm, _, _: &InputEvent, cx| {
                    this.error = None;
                    cx.notify();
                })
            })
            .collect();
        BranchForm {
            app: app.downgrade(),
            workspace,
            request,
            kind,
            title,
            focus: cx.focus_handle(),
            popup_host: None,
            fields,
            toggle,
            helper,
            error: None,
            _subscriptions: subscriptions,
        }
    });
    if let Some((_, input)) = form.read(cx).fields.first() {
        input.read(cx).focus_handle(cx).focus(window, cx);
    }
    (title, form)
}

impl BranchForm {
    fn close(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((interaction, popover)) = &self.popup_host {
            let _ = interaction.update(cx, |state, cx| {
                state.form = None;
                state.form_bounds = None;
                cx.notify();
            });
            if let Some(popover) = popover.as_ref().and_then(WeakEntity::upgrade) {
                popover.focus_handle(cx).focus(window, cx);
            }
            window.refresh();
        } else {
            window.close_dialog(cx);
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) -> bool {
        let values: Vec<_> = self
            .fields
            .iter()
            .map(|(_, field)| field.read(cx).value().to_string().trim().to_string())
            .collect();
        let action = match &self.kind {
            FormKind::Create => git::BranchAction::Create {
                name: values[0].clone(),
                checkout: self.toggle,
            },
            FormKind::Rename => git::BranchAction::Rename(values[0].clone()),
            FormKind::Push => git::BranchAction::Push {
                remote: values[0].clone(),
                destination: values[1].clone(),
                set_upstream: self.toggle,
            },
            FormKind::Worktree => git::BranchAction::Worktree {
                path: PathBuf::from(&values[0]),
                new_branch: (!values[1].is_empty()).then(|| values[1].clone()),
            },
            FormKind::Update(checkout) => {
                if *checkout {
                    git::BranchAction::CheckoutUpdate {
                        rebase: self.toggle,
                    }
                } else {
                    git::BranchAction::Update {
                        rebase: self.toggle,
                    }
                }
            }
            FormKind::Confirm(action) => action.clone(),
        };
        if values.first().is_some_and(|s| s.is_empty())
            || (matches!(self.kind, FormKind::Push) && values[1].is_empty())
        {
            self.error = Some("请填写必填字段".into());
            cx.notify();
            return false;
        }
        let request = git::BranchRequest {
            action,
            ..self.request.clone()
        };
        let started = self
            .app
            .update(cx, |this, cx| {
                this.start_branch_operation(self.workspace.clone(), request, cx)
            })
            .unwrap_or(false);
        if !started {
            self.error = Some("操作未启动，请检查工作区是否已切换或仍有任务运行".into());
            cx.notify();
        }
        started
    }
}

impl Render for BranchForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("branch-form")
            .debug_selector(|| "branch-form".into())
            .w_full()
            .gap_3()
            .text_size(px(13.))
            .when_some(self.popup_host.clone(), |body, (interaction, _)| {
                let bounds_state = interaction.clone();
                body.key_context("GitBranchForm")
                    .track_focus(&self.focus)
                    .w(px(
                        (f32::from(window.viewport_size().width) - 32.).clamp(240., 300.)
                    ))
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .text_color(rgb(palette().text))
                    .shadow_md()
                    .on_prepaint(move |bounds, _, cx| {
                        let _ =
                            bounds_state.update(cx, |state, _| state.form_bounds = Some(bounds));
                    })
                    .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close(window, cx)))
                    .on_action(cx.listener(|this, _: &CancelBranchForm, window, cx| {
                        cx.stop_propagation();
                        this.close(window, cx);
                    }))
                    .on_action(cx.listener(|this, _: &SubmitBranchForm, window, cx| {
                        cx.stop_propagation();
                        if this.submit(cx) {
                            this.close(window, cx);
                        }
                    }))
                    .child(div().font_semibold().child(self.title))
            })
            .child(
                div()
                    .text_color(rgb(palette().muted))
                    .child(self.helper.clone()),
            );
        for (index, (label, input)) in self.fields.iter().enumerate() {
            body = body.child(
                v_flex()
                    .gap_1()
                    .child(label.clone())
                    .child(Input::new(input).aria_label(label.clone()).h(px(32.))),
            );
            if index == 0 && matches!(self.kind, FormKind::Worktree) {
                body = body.child(
                    Button::new("worktree-browse")
                        .label("选择父目录…")
                        .small()
                        .ghost()
                        .on_click(cx.listener(|_, _, window, cx| {
                            let chosen = cx.prompt_for_paths(PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some("选择 Worktree 的父目录".into()),
                            });
                            cx.spawn_in(window, async move |this, cx| {
                                if let Ok(Ok(Some(paths))) = chosen.await
                                    && let Some(parent) = paths.first()
                                {
                                    let parent = parent.clone();
                                    let _ = this.update_in(cx, |this, window, cx| {
                                        let name = this.fields[1].1.read(cx).value().to_string();
                                        let name = name
                                            .rsplit('/')
                                            .next()
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or("worktree");
                                        this.fields[0].1.update(cx, |input, cx| {
                                            input.set_value(
                                                parent.join(name).to_string_lossy().into_owned(),
                                                window,
                                                cx,
                                            )
                                        });
                                    });
                                }
                            })
                            .detach();
                        })),
                );
            }
        }
        let toggle = match self.kind {
            FormKind::Create => Some(if self.toggle {
                "✓ 创建后切换"
            } else {
                "创建后切换"
            }),
            FormKind::Push => Some(if self.toggle {
                "✓ 设置为上游"
            } else {
                "设置为上游"
            }),
            FormKind::Update(_) => Some(if self.toggle {
                "更新方式：Rebase（点击切换为 Merge）"
            } else {
                "更新方式：Merge（点击切换为 Rebase）"
            }),
            _ => None,
        };
        if let Some(label) = toggle {
            body = body.child(
                Button::new("branch-form-toggle")
                    .label(label)
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle = !this.toggle;
                        cx.notify();
                    })),
            );
        }
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .id("branch-form-error")
                    .debug_selector(|| "branch-form-error".into())
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        body.child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("branch-form-cancel")
                        .debug_selector(|| "branch-form-cancel".into())
                        .label("取消")
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                )
                .child(
                    Button::new("branch-form-submit")
                        .debug_selector(|| "branch-form-submit".into())
                        .label(match self.kind {
                            FormKind::Create => "创建",
                            FormKind::Rename => "保存",
                            _ => "确认执行",
                        })
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.submit(cx) {
                                this.close(window, cx);
                            }
                        })),
                ),
        )
    }
}
