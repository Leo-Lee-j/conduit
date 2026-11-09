//! Conflict list and an aligned, editable three-way merge review.
use super::*;
use git::conflicts::{ConflictFile, Decision, MergeBlock, Resolution, ResolutionContent};
use gpui_kit::component::input::{Editor, EditorState};

fn tr(key: &str) -> String {
    conduit::i18n::text(key)
}

impl ConduitApp {
    pub(super) fn open_git_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        let paths = self.conflict_paths();
        let browser = if let Some(browser) = self.git.conflict_browsers.get(&workspace) {
            browser.clone()
        } else {
            let owner = cx.entity().downgrade();
            let path = workspace.clone();
            let browser = cx.new(|cx| ConflictBrowser {
                owner,
                workspace: path,
                paths: Vec::new(),
                drafts: HashMap::new(),
                selected: None,
                loading: false,
                saving: false,
                error: None,
                scroll: ScrollHandle::new(),
                current: 0,
                expanded: false,
                focus: cx.focus_handle(),
                cancelled: Arc::new(AtomicBool::new(false)),
                task: Task::ready(()),
            });
            self.git
                .conflict_browsers
                .insert(workspace.clone(), browser.clone());
            browser
        };
        browser.update(cx, |browser, cx| {
            browser.paths = paths.clone();
            cx.notify();
        });
        self.git.announced_conflicts.insert(workspace, paths);
        let focus = browser.read(cx).focus.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();
            let width = (f32::from(viewport.width) - 32.).clamp(280., 1600.);
            let height = (f32::from(viewport.height) - 48.).clamp(240., 1000.);
            let browser = browser.clone();
            dialog
                .entrance(gpui_kit::component::dialog::DialogEntrance::Fade)
                .w(px(width))
                .margin_top(px(24.))
                .p_0()
                .gap_0()
                .close_button(false)
                .overlay_closable(false)
                .content(move |content, _, _| {
                    content.child(div().w_full().h(px(height)).child(browser.clone()))
                })
        });
        focus.focus(window, cx);
    }

    fn conflict_paths(&self) -> Vec<String> {
        self.git
            .snapshot
            .as_ref()
            .map(|s| {
                s.files
                    .iter()
                    .filter(|file| file.conflicted())
                    .map(|file| file.path.clone())
                    .collect()
            })
            .unwrap_or_else(|| {
                self.git
                    .repository
                    .as_ref()
                    .map(|r| r.conflicts.clone())
                    .unwrap_or_default()
            })
    }

    pub(super) fn discover_git_conflicts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        let paths = self.conflict_paths();
        if let Some(browser) = self.git.conflict_browsers.get(&workspace) {
            browser.update(cx, |browser, cx| {
                browser.paths = paths.clone();
                cx.notify();
            });
        }
        if paths.is_empty() {
            self.git.announced_conflicts.remove(&workspace);
            return;
        }
        let previous = self.git.announced_conflicts.get(&workspace);
        let newly_conflicted =
            previous.is_none_or(|previous| paths.iter().any(|path| !previous.contains(path)));
        if newly_conflicted && self.git.operation.is_none() && !window.has_active_dialog(cx) {
            self.open_git_conflicts(window, cx);
        }
    }

    fn start_conflict_resolution(
        &mut self,
        workspace: PathBuf,
        resolution: Resolution,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.selected_workspace_path().as_ref() != Some(&workspace) {
            return Err(tr("git_conflicts.workspace_changed"));
        }
        if let Some(reason) = self.branch_operation_blocked(&workspace) {
            return Err(reason.into());
        }
        self.git.operation = Some(GitOperation {
            worktree: git::worktree_root(&workspace),
            probe: git::OperationProbe::start(
                workspace,
                git::Operation::ResolveConflict(Box::new(resolution)),
            ),
            draft: String::new(),
            commit_push: None,
            automatic_stage: false,
        });
        cx.notify();
        Ok(())
    }
}

struct BlockEditor {
    block: MergeBlock,
    panes: [Entity<EditorState>; 3],
    generated: String,
    manual: bool,
    reviewed: bool,
    _subscription: Subscription,
}
struct Draft {
    file: ConflictFile,
    blocks: Vec<BlockEditor>,
}

pub(super) struct ConflictBrowser {
    owner: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    paths: Vec<String>,
    drafts: HashMap<String, Draft>,
    selected: Option<String>,
    loading: bool,
    saving: bool,
    error: Option<String>,
    scroll: ScrollHandle,
    current: usize,
    expanded: bool,
    focus: FocusHandle,
    cancelled: Arc<AtomicBool>,
    task: Task<()>,
}

impl ConflictBrowser {
    fn busy(&self, cx: &App) -> bool {
        self.loading
            || self.saving
            || self.owner.upgrade().is_none_or(|owner| {
                let app = owner.read(cx);
                app.selected_workspace_path().as_ref() != Some(&self.workspace)
                    || app.branch_operation_blocked(&self.workspace).is_some()
            })
    }

    fn open_file(
        &mut self,
        path: String,
        quick: Option<ResolutionContent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loading || self.saving {
            return;
        }
        self.error = None;
        self.selected = Some(path.clone());
        self.current = 0;
        self.scroll = ScrollHandle::new();
        if self.drafts.contains_key(&path) {
            if let Some(content) = quick {
                self.save(content, cx);
            }
            cx.notify();
            return;
        }
        self.loading = true;
        self.cancelled.store(true, Ordering::Relaxed);
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        let ticket = cancelled.clone();
        let workspace = self.workspace.clone();
        self.task = cx.spawn_in(window, async move |this, cx| {
            let requested = path.clone();
            let result = cx
                .background_executor()
                .spawn(async move { git::conflicts::load(&workspace, &requested, &cancelled) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                if ticket.load(Ordering::Relaxed) {
                    return;
                }
                this.loading = false;
                match result {
                    Ok(file) => {
                        let mut blocks = Vec::new();
                        if let Some(plan) = &file.plan {
                            for (index, block) in plan.blocks.iter().cloned().enumerate() {
                                let generated = block.result();
                                let manual = block.worktree.is_some();
                                let extension = Path::new(&path)
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("");
                                let language = match extension {
                                    "rs" => "rust",
                                    "js" | "jsx" => "javascript",
                                    "ts" | "tsx" => "typescript",
                                    "py" => "python",
                                    "json" => "json",
                                    "java" => "java",
                                    "c" | "h" => "c",
                                    "cpp" | "hpp" => "cpp",
                                    "html" => "html",
                                    "css" => "css",
                                    "toml" => "toml",
                                    "yaml" | "yml" => "yaml",
                                    "md" => "markdown",
                                    _ => "plain_text",
                                };
                                let panes = [&block.ours, &generated, &block.theirs].map(|text| {
                                    cx.new(|cx| {
                                        let mut editor = EditorState::new(window, cx)
                                            .language(language)
                                            .soft_wrap(false)
                                            .line_number(true);
                                        editor.set_value(text.clone(), window, cx);
                                        editor
                                    })
                                });
                                let selected = path.clone();
                                let subscription = cx.subscribe(
                                    &panes[1],
                                    move |this, input, event: &InputEvent, cx| {
                                        if matches!(event, InputEvent::Change)
                                            && let Some(draft) = this.drafts.get_mut(&selected)
                                            && let Some(block) = draft.blocks.get_mut(index)
                                        {
                                            block.manual = block.block.worktree.is_some()
                                                || input.read(cx).value().as_ref()
                                                    != block.generated;
                                            if block.manual {
                                                block.reviewed = false;
                                            }
                                            cx.notify();
                                        }
                                    },
                                );
                                blocks.push(BlockEditor {
                                    block,
                                    panes,
                                    generated,
                                    manual,
                                    reviewed: false,
                                    _subscription: subscription,
                                });
                            }
                        }
                        this.drafts.insert(path, Draft { file, blocks });
                        if let Some(content) = quick {
                            this.save(content, cx);
                        }
                    }
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn draft(&self) -> Option<&Draft> {
        self.selected
            .as_ref()
            .and_then(|path| self.drafts.get(path))
    }

    fn side_label(&self, ours: bool, complete_file: bool) -> String {
        let label = tr(match (ours, complete_file) {
            (true, true) => "git_conflicts.accept_ours",
            (false, true) => "git_conflicts.accept_theirs",
            (true, false) => "git_conflicts.ours",
            (false, false) => "git_conflicts.theirs",
        });
        let Some(draft) = self.draft() else {
            return label;
        };
        let snapshot = &draft.file.snapshot;
        if snapshot.stages[if ours { 1 } else { 2 }].is_none() {
            return format!("{label} ({})", tr("git_conflicts.deleted"));
        }
        if complete_file {
            return label;
        }
        let revision = if ours {
            snapshot.context.head.as_deref().unwrap_or("")
        } else {
            &snapshot.incoming
        };
        let name = if ours {
            snapshot
                .context
                .current_ref
                .as_deref()
                .and_then(|s| s.strip_prefix("refs/heads/"))
                .unwrap_or("")
        } else {
            ""
        };
        format!(
            "{label} · {name} {}",
            revision.chars().take(8).collect::<String>()
        )
    }
    fn pending(&self) -> usize {
        self.draft()
            .map(|draft| {
                draft
                    .blocks
                    .iter()
                    .filter(|editor| {
                        if editor.manual {
                            !editor.reviewed && editor.block.changed
                        } else {
                            editor.block.pending()
                        }
                    })
                    .count()
            })
            .unwrap_or(0)
    }
    fn change(
        &mut self,
        index: usize,
        side: usize,
        decision: Decision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy(cx) {
            return;
        }
        let Some(draft) = self
            .selected
            .as_ref()
            .and_then(|path| self.drafts.get_mut(path))
        else {
            return;
        };
        let editor = &mut draft.blocks[index];
        if editor.manual {
            return;
        }
        editor.block.decisions[side] = if editor.block.decisions[side] == decision {
            Decision::Pending
        } else {
            decision
        };
        Self::sync(editor, window, cx);
        cx.notify();
    }
    fn sync(editor: &mut BlockEditor, window: &mut Window, cx: &mut Context<Self>) {
        editor.generated = editor.block.result();
        editor.manual = false;
        editor.reviewed = false;
        editor.panes[1].update(cx, |state, cx| {
            state.set_value(editor.generated.clone(), window, cx)
        });
    }
    fn apply_simple(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }
        if let Some(draft) = self
            .selected
            .as_ref()
            .and_then(|path| self.drafts.get_mut(path))
        {
            for editor in &mut draft.blocks {
                if !editor.manual && !editor.block.conflict && editor.block.pending() {
                    editor.block.apply_non_conflicting();
                    Self::sync(editor, window, cx);
                }
            }
        }
        cx.notify();
    }
    fn navigate(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(draft) = self.draft() else {
            return;
        };
        let changes: Vec<_> = draft
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.block.changed)
            .map(|(i, _)| i)
            .collect();
        if changes.is_empty() {
            return;
        }
        let next = if forward {
            changes
                .iter()
                .find(|i| **i > self.current)
                .copied()
                .unwrap_or(changes[0])
        } else {
            changes
                .iter()
                .rev()
                .find(|i| **i < self.current)
                .copied()
                .unwrap_or(*changes.last().unwrap())
        };
        self.current = next;
        self.scroll.scroll_to_item(next);
        cx.notify();
    }
    fn save_result(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.draft() else {
            return;
        };
        if self.pending() > 0 || draft.file.plan.is_none() {
            return;
        }
        let text = draft
            .blocks
            .iter()
            .map(|block| block.panes[1].read(cx).value().to_string())
            .collect::<String>();
        self.save(ResolutionContent::Merged(text), cx);
    }
    fn save(&mut self, content: ResolutionContent, cx: &mut Context<Self>) {
        if self.busy(cx) {
            return;
        }
        let Some(draft) = self.draft() else {
            return;
        };
        let resolution = Resolution {
            expected: draft.file.snapshot.clone(),
            content,
        };
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let workspace = self.workspace.clone();
        match owner.update(cx, |app, cx| {
            app.start_conflict_resolution(workspace, resolution, cx)
        }) {
            Ok(()) => {
                self.saving = true;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    pub(super) fn finished(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        if !self.saving {
            return;
        }
        self.saving = false;
        self.error = error;
        if self.error.is_none()
            && let Some(path) = self.selected.take()
        {
            self.paths.retain(|p| p != &path);
            self.drafts.remove(&path);
        }
        cx.notify();
    }

    fn file_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = v_flex()
            .id("conflict-files")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .gap_1();
        if self.paths.is_empty() {
            return body
                .child(div().p_4().child(tr("git_conflicts.all_resolved")))
                .into_any_element();
        }
        for (index, path) in self.paths.iter().enumerate() {
            let open = path.clone();
            let ours = path.clone();
            let theirs = path.clone();
            body = body.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new(SharedString::from(format!("conflict-file-{index}")))
                            .ghost()
                            .label(path.clone())
                            .debug_selector(move || format!("conflict-file-{index}"))
                            .flex_1()
                            .min_w_0()
                            .tooltip(path.clone())
                            .disabled(self.loading || self.saving)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_file(open.clone(), None, window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("conflict-ours-{index}")))
                            .small()
                            .label(tr("git_conflicts.accept_ours"))
                            .disabled(self.busy(cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_file(
                                    ours.clone(),
                                    Some(ResolutionContent::Ours),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("conflict-theirs-{index}")))
                            .small()
                            .label(tr("git_conflicts.accept_theirs"))
                            .disabled(self.busy(cx))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_file(
                                    theirs.clone(),
                                    Some(ResolutionContent::Theirs),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            );
        }
        body.into_any_element()
    }

    fn gutter(
        &self,
        index: usize,
        side: usize,
        editor: &BlockEditor,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let decision = editor.block.decisions[side];
        let changed = if side == 0 {
            editor.block.ours != editor.block.base
        } else {
            editor.block.theirs != editor.block.base
        };
        let disabled = self.busy(cx) || editor.manual || !changed;
        let label = if decision == Decision::Apply {
            tr("git_conflicts.undo_apply")
        } else {
            tr("git_conflicts.apply_side")
        };
        v_flex()
            .w(px(32.))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .py_1()
            .child(
                Button::new(SharedString::from(format!("merge-apply-{index}-{side}")))
                    .ghost()
                    .small()
                    .compact()
                    .icon(if decision == Decision::Apply {
                        IconName::Undo2
                    } else if side == 0 {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronLeft
                    })
                    .accessibility_label(label.clone())
                    .tooltip(label)
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(index, side, Decision::Apply, window, cx)
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("merge-ignore-{index}-{side}")))
                    .ghost()
                    .small()
                    .compact()
                    .icon(if decision == Decision::Ignore {
                        IconName::Undo2
                    } else {
                        IconName::Close
                    })
                    .accessibility_label(tr("git_conflicts.ignore_side"))
                    .tooltip(tr("git_conflicts.ignore_side"))
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.change(index, side, Decision::Ignore, window, cx)
                    })),
            )
            .into_any_element()
    }

    fn merge_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.draft() else {
            return div().flex_1().into_any_element();
        };
        if let Some(note) = &draft.file.note {
            return v_flex()
                .flex_1()
                .p_4()
                .gap_3()
                .child(note.clone())
                .child(tr("git_conflicts.choose_whole_file"))
                .into_any_element();
        }
        let mut body = v_flex()
            .id("merge-blocks")
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for (index, editor) in draft.blocks.iter().enumerate() {
            if !editor.block.changed
                && !editor.manual
                && !self.expanded
                && editor.block.base.lines().count() > 6
            {
                body = body.child(
                    Button::new(SharedString::from(format!("merge-context-{index}")))
                        .ghost()
                        .w_full()
                        .label(conduit::i18n::format(
                            "git_conflicts.unchanged_lines",
                            &[("count", editor.block.base.lines().count().to_string())],
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.expanded = true;
                            cx.notify();
                        })),
                );
                continue;
            }
            let result = editor.panes[1].read(cx).value();
            let lines = [&editor.block.ours, result.as_ref(), &editor.block.theirs]
                .iter()
                .map(|s| s.lines().count() + 1)
                .max()
                .unwrap_or(1);
            let height = (lines.clamp(2, 40) as f32 * 20. + 12.).max(54.);
            let unresolved = if editor.manual {
                !editor.reviewed
            } else {
                editor.block.pending()
            };
            let color = if unresolved && editor.block.conflict {
                palette().danger_border
            } else if unresolved {
                palette().warning_border
            } else {
                palette().border
            };
            let status = if editor.manual {
                tr("git_conflicts.manual")
            } else if editor.block.conflict {
                tr("git_conflicts.conflict")
            } else {
                tr("git_conflicts.change")
            };
            let pane = |slot: usize| {
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(height))
                    .when(editor.block.changed, |pane| {
                        pane.bg(rgb(if editor.block.conflict && unresolved {
                            palette().danger_bg
                        } else {
                            palette().warning_bg
                        }))
                    })
                    .child(
                        Editor::new(&editor.panes[slot])
                            .appearance(false)
                            .bordered(false)
                            .h(relative(1.))
                            .readonly(slot != 1 || self.saving)
                            .text_size(ui_text_size(12.))
                            .aria_label(tr(if slot == 0 {
                                "git_conflicts.ours"
                            } else if slot == 1 {
                                "git_conflicts.result"
                            } else {
                                "git_conflicts.theirs"
                            })),
                    )
            };
            let mut block = v_flex()
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(color));
            if editor.block.changed || editor.manual {
                block = block.child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .px_2()
                        .h(px(30.))
                        .bg(rgb(palette().surface_strong))
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .min_w_0()
                                .truncate()
                                .flex_1()
                                .child(format!(
                                    "{} · {}:{} / {}:{} / {}:{} · {}",
                                    status,
                                    tr("git_conflicts.ours"),
                                    editor.block.ours_line,
                                    tr("git_conflicts.base"),
                                    editor.block.base_line,
                                    tr("git_conflicts.theirs"),
                                    editor.block.theirs_line,
                                    tr(if unresolved {
                                        "git_conflicts.pending"
                                    } else {
                                        "git_conflicts.resolved"
                                    })
                                )),
                        )
                        .when(editor.manual && !editor.reviewed, |row| {
                            row.child(
                                Button::new(SharedString::from(format!("merge-review-{index}")))
                                    .debug_selector(move || format!("merge-review-{index}"))
                                    .small()
                                    .ghost()
                                    .label(tr("git_conflicts.mark_reviewed"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(draft) = this
                                            .selected
                                            .as_ref()
                                            .and_then(|p| this.drafts.get_mut(p))
                                        {
                                            draft.blocks[index].reviewed = true;
                                        }
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            Button::new(SharedString::from(format!("merge-reset-{index}")))
                                .small()
                                .ghost()
                                .icon(IconName::Undo2)
                                .tooltip(tr("git_conflicts.reset_block"))
                                .accessibility_label(tr("git_conflicts.reset_block"))
                                .disabled(self.busy(cx))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(draft) =
                                        this.selected.as_ref().and_then(|p| this.drafts.get_mut(p))
                                    {
                                        let editor = &mut draft.blocks[index];
                                        editor.block.reset();
                                        Self::sync(editor, window, cx);
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            }
            block = block.child(
                h_flex()
                    .w_full()
                    .items_start()
                    .child(pane(0))
                    .child(self.gutter(index, 0, editor, cx))
                    .child(pane(1))
                    .child(self.gutter(index, 1, editor, cx))
                    .child(pane(2)),
            );
            body = body.child(block);
        }
        body.into_any_element()
    }
}

impl Drop for ConflictBrowser {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Render for ConflictBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let detail = self.selected.is_some();
        let mut view = v_flex()
            .id("git-conflicts-dialog")
            .debug_selector(|| "git-conflicts-dialog".into())
            .key_context("GitDiffDialog")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .bg(rgb(palette().surface))
            .overflow_hidden()
            .on_action(cx.listener(|this, _: &NextDiff, _, cx| this.navigate(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousDiff, _, cx| this.navigate(false, cx)))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .p_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .when(detail, |row| {
                        row.child(
                            Button::new("conflict-back")
                                .ghost()
                                .small()
                                .icon(IconName::ChevronLeft)
                                .accessibility_label(tr("git_conflicts.back"))
                                .tooltip(tr("git_conflicts.back"))
                                .disabled(self.loading || self.saving)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.selected = None;
                                    this.error = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        div().flex_1().min_w_0().truncate().font_semibold().child(
                            self.selected
                                .clone()
                                .unwrap_or_else(|| tr("git_conflicts.title")),
                        ),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(11.))
                            .child(conduit::i18n::format(
                                "git_conflicts.files_remaining",
                                &[("count", self.paths.len().to_string())],
                            )),
                    )
                    .child(
                        Button::new("conflict-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .accessibility_label(tr("git_conflicts.close"))
                            .tooltip(tr("git_conflicts.drafts_kept"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            );
        if let Some(error) = &self.error {
            view = view.child(
                div()
                    .p_2()
                    .text_size(ui_text_size(12.))
                    .bg(rgb(palette().danger_bg))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if self.loading {
            return view.child(div().p_4().child(Spinner::new()));
        }
        if !detail {
            let rebase = self.owner.upgrade().is_some_and(|owner| {
                owner
                    .read(cx)
                    .git
                    .repository
                    .as_ref()
                    .is_some_and(|repo| repo.integration == Some(git::IntegrationState::Rebase))
            });
            return view
                .when(rebase, |view| {
                    view.child(
                        div()
                            .px_3()
                            .py_2()
                            .text_size(ui_text_size(11.))
                            .bg(rgb(palette().warning_bg))
                            .child(tr("git_conflicts.rebase_hint")),
                    )
                })
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(ui_text_size(12.))
                        .child(tr("git_conflicts.list_hint")),
                )
                .child(self.file_list(cx));
        }
        let ready = self.draft().is_some();
        let has_plan = self.draft().is_some_and(|draft| draft.file.plan.is_some());
        let rebase = self
            .draft()
            .is_some_and(|d| d.file.snapshot.integration == Some(git::IntegrationState::Rebase));
        let mut toolbar = h_flex()
            .w_full()
            .flex_wrap()
            .p_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(palette().border));
        toolbar = toolbar.child(
            Button::new("merge-simple")
                .debug_selector(|| "merge-simple".into())
                .small()
                .label(tr("git_conflicts.apply_simple"))
                .disabled(self.busy(cx) || !has_plan)
                .on_click(cx.listener(|this, _, window, cx| this.apply_simple(window, cx))),
        );
        for (id, label, forward) in [
            ("merge-previous", "git_conflicts.previous", false),
            ("merge-next", "git_conflicts.next", true),
        ] {
            toolbar = toolbar.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .label(tr(label))
                    .disabled(!has_plan)
                    .on_click(cx.listener(move |this, _, _, cx| this.navigate(forward, cx))),
            );
        }
        toolbar = toolbar.child(
            Button::new("merge-context")
                .small()
                .ghost()
                .label(tr("git_conflicts.toggle_context"))
                .disabled(!has_plan)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.expanded = !this.expanded;
                    cx.notify();
                })),
        );
        view = view.child(toolbar);
        if self.draft().is_some_and(|draft| {
            draft
                .blocks
                .iter()
                .any(|block| block.block.worktree.is_some())
        }) {
            view = view.child(
                div()
                    .p_2()
                    .text_size(ui_text_size(11.))
                    .bg(rgb(palette().warning_bg))
                    .child(tr("git_conflicts.worktree_preserved")),
            );
        }
        if rebase {
            view = view.child(
                div()
                    .p_2()
                    .text_size(ui_text_size(11.))
                    .bg(rgb(palette().warning_bg))
                    .child(tr("git_conflicts.rebase_hint")),
            );
        }
        if has_plan {
            view = view.child(
                h_flex()
                    .w_full()
                    .px_2()
                    .py_2()
                    .gap_2()
                    .bg(rgb(palette().surface_strong))
                    .children(
                        [
                            self.side_label(true, false),
                            tr("git_conflicts.result"),
                            self.side_label(false, false),
                        ]
                        .map(|label| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_center()
                                .text_size(ui_text_size(12.))
                                .font_semibold()
                                .child(label)
                        }),
                    ),
            );
        }
        view = view.child(self.merge_body(cx));
        view.child(
            h_flex()
                .w_full()
                .flex_wrap()
                .p_2()
                .gap_2()
                .items_center()
                .border_t_1()
                .border_color(rgb(palette().border))
                .child(
                    Button::new("merge-accept-ours")
                        .small()
                        .label(self.side_label(true, true))
                        .disabled(self.busy(cx) || !ready)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.save(ResolutionContent::Ours, cx)),
                        ),
                )
                .child(
                    Button::new("merge-accept-theirs")
                        .small()
                        .label(self.side_label(false, true))
                        .disabled(self.busy(cx) || !ready)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.save(ResolutionContent::Theirs, cx)),
                        ),
                )
                .child(
                    Button::new("merge-reload")
                        .small()
                        .ghost()
                        .label(tr("git_conflicts.reload"))
                        .tooltip(tr("git_conflicts.reload_hint"))
                        .disabled(self.busy(cx))
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(path) = this.selected.clone() {
                                this.drafts.remove(&path);
                                this.open_file(path, None, window, cx);
                            }
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(ui_text_size(11.))
                        .child(conduit::i18n::format(
                            "git_conflicts.blocks_remaining",
                            &[("count", self.pending().to_string())],
                        )),
                )
                .child(
                    Button::new("merge-save")
                        .debug_selector(|| "merge-save".into())
                        .primary()
                        .small()
                        .label(tr("git_conflicts.save"))
                        .disabled(self.busy(cx) || !has_plan || self.pending() > 0)
                        .on_click(cx.listener(|this, _, _, cx| this.save_result(cx))),
                ),
        )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
