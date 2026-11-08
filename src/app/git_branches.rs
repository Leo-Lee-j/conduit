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
        .text_size(ui_text_size(11.))
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
                    .unwrap_or(conduit::i18n::text_static("git_branches.untrack"))
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
        menu = menu.item(PopupMenuItem::label(conduit::i18n::format(
            "git_branches.in_use_by_worktree",
            &[(
                "arg0",
                format!("{}", selected.occupied.as_ref().unwrap().display()),
            )],
        )));
    } else if blocked {
        menu = menu.item(PopupMenuItem::label(conduit::i18n::text_static(
            "git_branches.a_task_is_running_in_this_repository",
        )));
    }
    if in_progress {
        menu = menu.item(PopupMenuItem::label(conduit::i18n::text_static(
            "git_branches.continue_or_abort_the_git_operation_in",
        )));
    }
    menu
}

fn fs_canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

impl ConduitApp {
    fn branch_operation_blocked(&self, path: &Path) -> Option<&'static str> {
        if self.closing || !self.writable {
            Some(conduit::i18n::text_static(
                "git_branches.the_application_is_currently_read_only",
            ))
        } else if self.git.operation.is_some() {
            Some(conduit::i18n::text_static(
                "git_branches.a_git_operation_is_already_running_wait",
            ))
        } else if self.git.generation.is_some() {
            Some(conduit::i18n::text_static(
                "git_branches.finish_or_cancel_commit_message_generation_first",
            ))
        } else if self.worktree_has_runs(path) {
            Some(conduit::i18n::text_static(
                "git_branches.a_cli_task_is_running_in_this",
            ))
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
            self.git_feedback(path, reason, true, cx);
            return false;
        }
        if self.selected_workspace_path().as_ref() != Some(&path) {
            self.git_feedback(
                path,
                conduit::i18n::text_static(
                    "git_branches.the_workspace_has_changed_reopen_the_branch",
                ),
                true,
                cx,
            );
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
            commit_push: None,
            automatic_stage: false,
        });
        self.git_feedback(
            path,
            conduit::i18n::text_static("git_branches.running_git_branch_operation"),
            false,
            cx,
        );
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
            .text_size(ui_text_size(12.))
            .text_color(rgb(palette().warning_text))
            .child(conduit::i18n::format(
                "git_branches.in_progress_conflicted_files",
                &[
                    ("arg0", state.label().to_string()),
                    ("arg1", format!("{}", repository.conflicts.len())),
                ],
            ))
            .child(div().text_size(ui_text_size(11.)).child(if supported {
                conduit::i18n::text_static(
                    "git_branches.resolve_conflicts_in_an_external_editor_run",
                )
            } else {
                conduit::i18n::text_static("git_branches.finish_or_abort_this_operation_in_the")
            }));
        for conflict in repository.conflicts.iter().take(20) {
            banner = banner.child(div().truncate().child(conflict.clone()));
        }
        if repository.conflicts.len() > 20 {
            banner = banner.child(conduit::i18n::text_static(
                "git_branches.run_git_status_in_the_terminal_to",
            ));
        }
        if supported {
            let mut buttons = h_flex().gap_2();
            for (label, action) in [
                (
                    conduit::i18n::text_static("git_branches.continue"),
                    git::BranchAction::Continue(state),
                ),
                (
                    conduit::i18n::text_static("git_branches.abort"),
                    git::BranchAction::Abort(state),
                ),
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
            conduit::i18n::text_static("git_branches.new_branch"),
            conduit::i18n::format(
                "git_branches.create_a_branch_from",
                &[("selected", selected.to_string())],
            ),
            vec![(
                conduit::i18n::text_static("git_branches.branch_name"),
                String::new(),
            )],
            true,
        ),
        FormKind::Rename => (
            conduit::i18n::text_static("git_branches.rename_branch"),
            conduit::i18n::text_static("git_branches.only_the_local_branch_will_be_renamed").into(),
            vec![(
                conduit::i18n::text_static("git_branches.branch_name"),
                selected.into(),
            )],
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
                conduit::i18n::text_static("git_branches.push_branch"),
                conduit::i18n::format(
                    "git_branches.push_available_remotes",
                    &[
                        ("selected", selected.to_string()),
                        ("arg0", repository.remotes.join(", ").to_string()),
                    ],
                ),
                vec![
                    (conduit::i18n::text_static("git_branches.remote"), remote),
                    (
                        conduit::i18n::text_static("git_branches.target_branch"),
                        destination,
                    ),
                ],
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
                conduit::i18n::text_static("git_branches.create_worktree"),
                conduit::i18n::format(
                    "git_branches.create_a_separate_working_directory_from_and",
                    &[("selected", selected.to_string())],
                ),
                vec![
                    (
                        conduit::i18n::text_static("git_branches.target_directory_absolute_path"),
                        path.to_string_lossy().into_owned(),
                    ),
                    (
                        conduit::i18n::text_static("git_branches.new_branch_name_optional"),
                        branch_name,
                    ),
                ],
                false,
            )
        }
        FormKind::Update(checkout) => (
            if *checkout {
                conduit::i18n::text_static("git_branches.switch_and_update_branch")
            } else {
                conduit::i18n::text_static("git_branches.update_branch")
            },
            conduit::i18n::format(
                "git_branches.update_the_upstream_of_only_fast_forward",
                &[
                    (
                        "arg0",
                        (if *checkout {
                            conduit::i18n::text_static("git_branches.switch_first_then")
                        } else {
                            ""
                        })
                        .to_string(),
                    ),
                    ("selected", selected.to_string()),
                ],
            ),
            vec![],
            false,
        ),
        FormKind::Confirm(action) => {
            let (title, helper) = match action {
                git::BranchAction::Delete => (
                    conduit::i18n::text_static("git_branches.delete_local_branch"),
                    conduit::i18n::format(
                        "git_branches.delete_normal_deletion_rejects_unmerged_branches_the",
                        &[("selected", selected.to_string())],
                    ),
                ),
                git::BranchAction::Merge => (
                    conduit::i18n::text_static("git_branches.merge_branch"),
                    conduit::i18n::format(
                        "git_branches.merge_into_if_conflicts_occur_resolve_them",
                        &[
                            ("selected", selected.to_string()),
                            ("arg0", repository.current.to_string()),
                        ],
                    ),
                ),
                git::BranchAction::Rebase => (
                    conduit::i18n::text_static("git_branches.rebase_current_branch"),
                    conduit::i18n::format(
                        "git_branches.rebase_onto_this_rewrites_the_current_branch",
                        &[
                            ("arg0", repository.current.to_string()),
                            ("selected", selected.to_string()),
                        ],
                    ),
                ),
                git::BranchAction::CheckoutRebase => (
                    conduit::i18n::text_static("git_branches.switch_and_rebase"),
                    conduit::i18n::format(
                        "git_branches.switch_to_then_rebase_it_onto_this",
                        &[
                            ("selected", selected.to_string()),
                            ("arg0", repository.current.to_string()),
                        ],
                    ),
                ),
                git::BranchAction::Abort(state) => (
                    conduit::i18n::text_static("git_branches.abort_git_operation"),
                    conduit::i18n::format(
                        "git_branches.abort_the_ongoing_git_will_try_to",
                        &[("arg0", state.label().to_string())],
                    ),
                ),
                _ => (
                    conduit::i18n::text_static("git_branches.git_operation"),
                    conduit::i18n::text_static("git_branches.run_this_operation").into(),
                ),
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
            self.error = Some(
                conduit::i18n::text_static("git_branches.complete_the_required_fields").into(),
            );
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
            self.error = Some(
                conduit::i18n::text_static(
                    "git_branches.operation_did_not_start_check_whether_the",
                )
                .into(),
            );
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
            .text_size(ui_text_size(13.))
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
                        .label(conduit::i18n::text_static(
                            "git_branches.choose_parent_directory",
                        ))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|_, _, window, cx| {
                            let chosen = cx.prompt_for_paths(PathPromptOptions {
                                files: false,
                                directories: true,
                                multiple: false,
                                prompt: Some(
                                    conduit::i18n::text_static(
                                        "git_branches.choose_the_parent_directory_for_the_worktree",
                                    )
                                    .into(),
                                ),
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
                conduit::i18n::text_static("git_branches.switch_after_creation")
            } else {
                conduit::i18n::text_static("git_branches.switch_after_creation_77d27c")
            }),
            FormKind::Push => Some(if self.toggle {
                conduit::i18n::text_static("git_branches.set_upstream")
            } else {
                conduit::i18n::text_static("git_branches.set_upstream_7cb6a8")
            }),
            FormKind::Update(_) => Some(if self.toggle {
                conduit::i18n::text_static("git_branches.update_method_rebase_click_to_use_merge")
            } else {
                conduit::i18n::text_static("git_branches.update_method_merge_click_to_use_rebase")
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
                        .label(conduit::i18n::text_static("git_branches.cancel"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                )
                .child(
                    Button::new("branch-form-submit")
                        .debug_selector(|| "branch-form-submit".into())
                        .label(match self.kind {
                            FormKind::Create => conduit::i18n::text_static("git_branches.create"),
                            FormKind::Rename => conduit::i18n::text_static("git_branches.save"),
                            _ => conduit::i18n::text_static("git_branches.confirm"),
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
