//! Conflict list and an aligned, editable three-way merge review.
use super::*;
use git::conflicts::{ConflictFile, Decision, MergeBlock, Resolution, ResolutionContent};
use gpui_kit::component::input::{Editor, EditorState};

mod view;

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
            if browser
                .selected
                .as_ref()
                .is_none_or(|path| !paths.contains(path))
            {
                browser.selected = None;
                if let Some(path) = paths.first() {
                    browser.open_file(path.clone(), None, window, cx);
                }
            }
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
                                            .line_number(false);
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
                        this.current = blocks
                            .iter()
                            .position(|editor| editor.block.changed)
                            .unwrap_or(0);
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
    pub(super) fn finished(
        &mut self,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            if let Some(next) = self.paths.first().cloned() {
                self.open_file(next, None, window, cx);
            }
        }
        cx.notify();
    }
}

impl Drop for ConflictBrowser {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
