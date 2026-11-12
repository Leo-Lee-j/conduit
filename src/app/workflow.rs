//! Project task and preset dialogs. Every model invocation starts from a draft.
use super::*;
use crate::workflow::{self, Criterion, Preset, TaskStatus, WorkItem};
mod branching;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Tasks,
    Presets,
    Handoff,
}

pub(super) struct SessionDraft<'a> {
    pub workspace: &'a Path,
    pub provider: Provider,
    pub title: &'a str,
    pub prompt: &'a str,
    pub preset: Option<&'a Preset>,
}

struct WorkflowPanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    page: Page,
    title: Entity<InputState>,
    goal: Entity<TextareaState>,
    criteria: Entity<TextareaState>,
    preset_name: Entity<InputState>,
    preset_prompt: Entity<TextareaState>,
    through: Entity<InputState>,
    context: Entity<TextareaState>,
    source_session: Option<u64>,
    editing: Option<u64>,
    show_archived: bool,
    error: Option<String>,
    shown: usize,
    _subscriptions: Vec<Subscription>,
}

impl ConduitApp {
    pub(super) fn workflow_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("project-workflow")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::ListTodo)
            .accessibility_label(label("workflow.title"))
            .tooltip(label("workflow.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| this.open_workflow(window, cx)))
    }

    fn open_workflow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_workflow_at(None, None, window, cx);
    }

    pub(super) fn open_workflow_task(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_workflow_at(None, Some(id), window, cx);
    }

    pub(super) fn open_handoff_at(
        &mut self,
        session: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_session != Some(session) {
            return;
        }
        self.open_workflow_at(Some(index), None, window, cx);
    }

    fn open_workflow_at(
        &mut self,
        message: Option<usize>,
        task: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_task = task
            .and_then(|id| self.store.workflow.tasks.iter().find(|t| t.id == id))
            .cloned();
        if task.is_some() && selected_task.is_none() {
            return;
        }
        let Some(workspace) = selected_task
            .as_ref()
            .map(|task| task.workspace.clone())
            .or_else(|| self.selected_workspace_path())
        else {
            return;
        };
        let archived = selected_task.as_ref().is_some_and(|task| task.archived);
        let source_session = (self.selected_workspace_path().as_ref() == Some(&workspace))
            .then_some(self.selected_session)
            .flatten();
        let through = source_session
            .and_then(|id| self.session(id))
            .map_or(0, |s| message.map_or(s.messages.len(), |i| i + 1));
        // The app entity is already being updated by the message click handler.
        // Prepare the initial preview from `self` before the panel reads its host.
        let handoff = message.map(|index| {
            let session = source_session
                .and_then(|id| self.session(id))
                .ok_or_else(|| label("workflow.select_session").to_owned())?;
            workflow::handoff(&workspace, session, index)
        });
        let host = cx.entity();
        let view = cx.new(|cx| {
            let title =
                cx.new(|cx| InputState::new(window, cx).placeholder(label("workflow.name")));
            let goal =
                cx.new(|cx| TextareaState::new(window, cx).placeholder(label("workflow.goal")));
            let criteria = cx.new(|cx| {
                TextareaState::new(window, cx).placeholder(label("workflow.criteria_hint"))
            });
            let preset_name =
                cx.new(|cx| InputState::new(window, cx).placeholder(label("workflow.name")));
            let preset_prompt =
                cx.new(|cx| TextareaState::new(window, cx).placeholder(label("workflow.goal")));
            let through =
                cx.new(|cx| InputState::new(window, cx).default_value(through.to_string()));
            let context = cx.new(|cx| {
                TextareaState::new(window, cx).placeholder(label("workflow.context_hint"))
            });
            let subscriptions = vec![
                cx.observe(&host, |_, _, cx| cx.notify()),
                cx.observe(&through, |_, _, cx| cx.notify()),
            ];
            WorkflowPanel {
                host: host.downgrade(),
                workspace,
                page: if message.is_some() {
                    Page::Handoff
                } else {
                    Page::Tasks
                },
                title,
                goal,
                criteria,
                preset_name,
                preset_prompt,
                through,
                context,
                source_session,
                editing: None,
                show_archived: archived,
                error: None,
                shown: 30,
                _subscriptions: subscriptions,
            }
        });
        if let Some(handoff) = handoff {
            view.update(cx, |panel, cx| panel.apply_handoff(handoff, window, cx));
        }
        if selected_task.is_some() {
            view.update(cx, |panel, cx| panel.fill_task(selected_task, window, cx));
        }
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            dialog
                .title(label("workflow.title"))
                .w(px(
                    (f32::from(window.viewport_size().width) - 48.).clamp(280., 950.)
                ))
                .content(move |content, _, _| content.child(view.clone()))
        });
    }

    pub(super) fn workflow_session(
        &mut self,
        draft: SessionDraft<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        let SessionDraft {
            workspace,
            provider,
            title,
            prompt,
            preset,
        } = draft;
        if !self.writable
            || self.closing
            || self.composer_save_pending()
            || !self.store.cli.enabled(provider)
        {
            return Err(label("workflow.unavailable").into());
        }
        if prompt.trim().is_empty() || prompt.len() > workflow::MAX_CONTEXT_BYTES {
            return Err(label("workflow.context_limit").into());
        }
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| w.path == workspace)
            .ok_or_else(|| label("workflow.missing").to_owned())?;
        if let Some(preset) = preset {
            if preset.workspace != workspace || preset.provider != provider || preset.archived {
                return Err(label("workflow.missing").into());
            }
            if let ModelChannel::Provider(id) = &preset.channel
                && !self
                    .store
                    .providers
                    .iter()
                    .any(|p| &p.id == id && p.engine == provider)
            {
                return Err(label("workflow.service_missing").into());
            }
        }
        let previous_workspace = self.selected_workspace;
        let previous_session = self.selected_session;
        let id = self
            .create_session(index, provider, cx)
            .ok_or_else(|| label("workflow.unavailable").to_owned())?;
        if let Some(session) = self.session_mut(id) {
            if let Some(preset) = preset {
                preset.apply(session);
            }
            session.title = title.chars().take(120).collect();
        }
        self.selected_workspace = previous_workspace;
        self.selected_session = previous_session;
        self.switch(index, Some(id), window, cx);
        self.input.update(cx, |input, cx| {
            input.set_value(prompt.to_owned(), window, cx)
        });
        self.drafts.insert(id, prompt.to_owned());
        self.settings_ui.open = false;
        self.persist();
        cx.notify();
        Ok(id)
    }
}

impl WorkflowPanel {
    fn host(&self) -> Result<Entity<ConduitApp>, String> {
        self.host
            .upgrade()
            .ok_or_else(|| label("workflow.missing").into())
    }

    fn write<R>(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ConduitApp, &mut Context<ConduitApp>) -> Result<R, String>,
    ) -> Result<R, String> {
        self.host()?.update(cx, |app, cx| {
            if !app.writable
                || app.closing
                || !app
                    .store
                    .workspaces
                    .iter()
                    .any(|w| w.path == self.workspace)
            {
                return Err(label("workflow.unavailable").into());
            }
            let result = f(app, cx)?;
            app.persist();
            cx.notify();
            Ok(result)
        })
    }

    fn result<R>(&mut self, result: Result<R, String>, cx: &mut Context<Self>) {
        self.error = result.err();
        cx.notify();
    }

    fn save_task(&mut self, cx: &mut Context<Self>) {
        let title = self.title.read(cx).value().to_string();
        let goal = self.goal.read(cx).value().to_string();
        let criteria: Vec<_> = self
            .criteria
            .read(cx)
            .value()
            .lines()
            .filter(|s| !s.trim().is_empty())
            .map(|s| Criterion {
                text: s.trim().to_owned(),
                done: false,
            })
            .collect();
        let result = self.write(cx, |app, _| {
            let old = self
                .editing
                .and_then(|id| {
                    app.store
                        .workflow
                        .tasks
                        .iter()
                        .find(|t| t.id == id && t.workspace == self.workspace)
                })
                .cloned();
            if self.editing.is_some() && old.is_none() {
                return Err(label("workflow.missing").into());
            }
            let mut task = old.unwrap_or_else(|| WorkItem {
                id: 0,
                workspace: self.workspace.clone(),
                title: String::new(),
                goal: String::new(),
                branch: (app.git.workspace.as_ref() == Some(&self.workspace))
                    .then(|| app.git.repository.as_ref().map(|r| r.current.clone()))
                    .flatten(),
                sessions: vec![],
                criteria: vec![],
                status: TaskStatus::Planned,
                archived: false,
                created_at: unix_now(),
                origin: None,
                check_source: None,
            });
            let criteria = criteria
                .into_iter()
                .map(|mut c| {
                    c.done = task
                        .criteria
                        .iter()
                        .any(|old| old.text == c.text && old.done);
                    c
                })
                .collect();
            task.title = title;
            task.goal = goal;
            task.criteria = criteria;
            app.store.workflow.save_task(task)
        });
        if let Ok(id) = &result {
            self.editing = Some(*id);
        }
        self.result(result, cx);
    }

    fn edit_task(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let task = self.host().ok().and_then(|host| {
            host.read(cx)
                .store
                .workflow
                .tasks
                .iter()
                .find(|t| Some(t.id) == id && t.workspace == self.workspace)
                .cloned()
        });
        self.fill_task(task, window, cx);
    }

    fn fill_task(&mut self, task: Option<WorkItem>, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = task.as_ref().map(|t| t.id);
        self.error = None;
        self.title.update(cx, |input, cx| {
            input.set_value(
                task.as_ref().map_or(String::new(), |t| t.title.clone()),
                window,
                cx,
            )
        });
        self.goal.update(cx, |input, cx| {
            input.set_value(
                task.as_ref().map_or(String::new(), |t| t.goal.clone()),
                window,
                cx,
            )
        });
        self.criteria.update(cx, |input, cx| {
            input.set_value(
                task.as_ref().map_or(String::new(), |t| {
                    t.criteria
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                }),
                window,
                cx,
            )
        });
        cx.notify();
    }

    fn update_task(
        &mut self,
        id: u64,
        f: impl FnOnce(&mut WorkItem) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let result = self.write(cx, |app, _| {
            let task = app
                .store
                .workflow
                .tasks
                .iter_mut()
                .find(|t| t.id == id && t.workspace == self.workspace)
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            f(task)
        });
        self.result(result, cx);
    }

    fn link_session(&mut self, id: u64, cx: &mut Context<Self>) {
        let result = self.write(cx, |app, _| {
            let session = app
                .selected_session
                .filter(|_| app.selected_workspace_path().as_ref() == Some(&self.workspace))
                .ok_or_else(|| label("workflow.select_session").to_owned())?;
            let task = app
                .store
                .workflow
                .tasks
                .iter_mut()
                .find(|t| t.id == id && t.workspace == self.workspace)
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            if !task.sessions.contains(&session) {
                task.sessions.push(session);
            }
            Ok(())
        });
        self.result(result, cx);
    }

    fn start_task(
        &mut self,
        id: u64,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.write(cx, |app, cx| {
            let task = app
                .store
                .workflow
                .tasks
                .iter()
                .find(|t| t.id == id && t.workspace == self.workspace && !t.archived)
                .cloned()
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            let session = app.workflow_session(
                SessionDraft {
                    workspace: &self.workspace,
                    provider,
                    title: &task.title,
                    prompt: &task.prompt(),
                    preset: None,
                },
                window,
                cx,
            )?;
            let task = app
                .store
                .workflow
                .tasks
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap();
            task.sessions.push(session);
            Ok(())
        });
        if result.is_ok() {
            window.close_dialog(cx);
        }
        self.result(result, cx);
    }

    fn save_preset(&mut self, cx: &mut Context<Self>) {
        let name = self.preset_name.read(cx).value().to_string();
        let prompt = self.preset_prompt.read(cx).value().to_string();
        let result = self.write(cx, |app, _| {
            let session = self.source_session.and_then(|id| app.session(id));
            let selection = session
                .map(ComposerSelection::from_session)
                .unwrap_or_else(|| app.default_selection(app.new_session_provider));
            let permission = session.map_or(app.pending_permission, |s| s.permission);
            app.store.workflow.save_preset(Preset {
                id: 0,
                workspace: self.workspace.clone(),
                name,
                prompt,
                provider: selection.engine,
                channel: selection.channel,
                model: selection.model,
                effort: selection.effort,
                acp_variant: selection.opencode.variant,
                permission,
                archived: false,
            })
        });
        self.result(result, cx);
    }

    fn use_preset(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.write(cx, |app, cx| {
            let preset = app
                .store
                .workflow
                .presets
                .iter()
                .find(|p| p.id == id && p.workspace == self.workspace && !p.archived)
                .cloned()
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            app.workflow_session(
                SessionDraft {
                    workspace: &self.workspace,
                    provider: preset.provider,
                    title: &preset.name,
                    prompt: &preset.prompt,
                    preset: Some(&preset),
                },
                window,
                cx,
            )
        });
        if result.is_ok() {
            window.close_dialog(cx);
        }
        self.result(result, cx);
    }

    fn prepare_handoff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.host().and_then(|host| {
            let app = host.read(cx);
            let session = self
                .source_session
                .and_then(|id| {
                    app.store
                        .workspaces
                        .iter()
                        .find(|w| w.path == self.workspace)?
                        .sessions
                        .iter()
                        .find(|s| s.id == id)
                })
                .ok_or_else(|| label("workflow.select_session").to_owned())?;
            let index = self
                .through
                .read(cx)
                .value()
                .parse::<usize>()
                .ok()
                .and_then(|i| i.checked_sub(1))
                .ok_or_else(|| label("workflow.invalid_message").to_owned())?;
            workflow::handoff(&self.workspace, session, index)
        });
        self.apply_handoff(result, window, cx);
    }

    fn apply_handoff(
        &mut self,
        result: Result<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Ok(text) = &result {
            self.context
                .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
        self.result(result, cx);
    }

    fn send_handoff(&mut self, provider: Provider, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = self.context.read(cx).value().to_string();
        let result = self.write(cx, |app, cx| {
            app.workflow_session(
                SessionDraft {
                    workspace: &self.workspace,
                    provider,
                    title: label("workflow.handoff_session"),
                    prompt: &prompt,
                    preset: None,
                },
                window,
                cx,
            )
        });
        if result.is_ok() {
            window.close_dialog(cx);
        }
        self.result(result, cx);
    }

    fn task_page(&self, writable: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut tasks = self
            .host()
            .ok()
            .map(|host| {
                host.read(cx)
                    .store
                    .workflow
                    .tasks
                    .iter()
                    .filter(|t| t.workspace == self.workspace && t.archived == self.show_archived)
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        tasks.sort_by_key(|task| Some(task.id) != self.editing);
        let mut page = v_flex()
            .gap_2()
            .child(Input::new(&self.title))
            .child(Textarea::new(&self.goal).h(px(70.)))
            .child(Textarea::new(&self.criteria).h(px(60.)))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("task-save")
                            .label(label("workflow.save_task"))
                            .small()
                            .disabled(!writable)
                            .on_click(cx.listener(|this, _, _, cx| this.save_task(cx))),
                    )
                    .child(
                        Button::new("task-new")
                            .label(label("workflow.new_task"))
                            .small()
                            .ghost()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.edit_task(None, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("task-archive-filter")
                            .label(label("workflow.archived"))
                            .small()
                            .ghost()
                            .selected(self.show_archived)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_archived = !this.show_archived;
                                this.shown = 30;
                                cx.notify();
                            })),
                    ),
            );
        if tasks.is_empty() {
            page = page.child(label("workflow.no_tasks"));
        }
        for task in tasks.iter().take(self.shown) {
            let id = task.id;
            let owner = cx.weak_entity();
            let engines_owner = owner.clone();
            let mut card = v_flex()
                .id(("work-item", id))
                .p_2()
                .gap_1()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .child(format!("#{} · {}", id, task.title)),
                )
                .child(div().text_size(ui_text_size(12.)).child(task.goal.clone()));
            if let Some(branch) = &task.branch {
                card = card.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(branch.clone()),
                );
            }
            for (index, criterion) in task.criteria.iter().enumerate() {
                card = card.child(
                    Button::new((SharedString::from(format!("task-{id}-criterion")), index))
                        .small()
                        .ghost()
                        .label(format!(
                            "{} {}",
                            if criterion.done { "☑" } else { "☐" },
                            criterion.text
                        ))
                        .disabled(!writable || task.archived)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.update_task(
                                id,
                                |task| {
                                    if let Some(c) = task.criteria.get_mut(index) {
                                        c.done = !c.done;
                                        if !c.done && task.status == TaskStatus::Done {
                                            task.status = TaskStatus::Review;
                                        }
                                    }
                                    Ok(())
                                },
                                cx,
                            )
                        })),
                );
            }
            let workspace = self.workspace.clone();
            if let Ok(host) = self.host() {
                for session in &task.sessions {
                    let session = *session;
                    let weak = host.downgrade();
                    let workspace = workspace.clone();
                    let title = host.read(cx).session(session).map(|s| s.title.clone());
                    card = card.child(
                        Button::new(SharedString::from(format!("task-{id}-session-{session}")))
                            .small()
                            .ghost()
                            .label(title.unwrap_or_else(|| {
                                format!("#{session} · {}", label("workflow.missing"))
                            }))
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |app, cx| {
                                    if let Some(index) = app.store.workspaces.iter().position(|w| {
                                        w.path == workspace
                                            && w.sessions.iter().any(|s| s.id == session)
                                    }) {
                                        app.switch(index, Some(session), window, cx);
                                        window.close_dialog(cx);
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
            }
            card = card.child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new(("task-status", id))
                            .small()
                            .label(task.status.label())
                            .disabled(!writable || task.archived)
                            .dropdown_menu(move |mut menu, _, _| {
                                for status in TaskStatus::ALL {
                                    let owner = owner.clone();
                                    menu = menu.item(PopupMenuItem::new(status.label()).on_click(
                                        move |_, _, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.update_task(id, |t| t.set_status(status), cx)
                                            });
                                        },
                                    ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new(("task-edit", id))
                            .small()
                            .ghost()
                            .label(label("workflow.edit"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_task(Some(id), window, cx)
                            })),
                    )
                    .child(
                        Button::new(("task-link", id))
                            .small()
                            .ghost()
                            .label(label("workflow.link_session"))
                            .disabled(!writable || task.archived)
                            .on_click(cx.listener(move |this, _, _, cx| this.link_session(id, cx))),
                    )
                    .child(
                        Button::new(("task-session", id))
                            .small()
                            .ghost()
                            .label(label("workflow.prepare_session"))
                            .disabled(!writable || task.archived)
                            .dropdown_menu(move |mut menu, _, _| {
                                for provider in Provider::ALL {
                                    let owner = engines_owner.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(provider.label()).on_click(
                                            move |_, window, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.start_task(id, provider, window, cx)
                                                });
                                            },
                                        ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new(("task-worktree", id))
                            .small()
                            .ghost()
                            .label(label("workflow.worktree"))
                            .disabled(!writable || task.archived)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(host) = this.host.upgrade() {
                                    let workspace = this.workspace.clone();
                                    this.error = host
                                        .update(cx, |app, cx| {
                                            app.task_worktree(id, &workspace, window, cx)
                                        })
                                        .err();
                                }
                                cx.notify();
                            })),
                    )
                    .when(task.archived && task.origin.is_some(), |row| {
                        row.child(
                            Button::new(("task-cleanup", id))
                                .small()
                                .ghost()
                                .label(label("workflow.cleanup"))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(host) = this.host.upgrade() {
                                        host.update(cx, |app, cx| {
                                            app.cleanup_task_worktree(id, window, cx)
                                        });
                                    }
                                })),
                        )
                    })
                    .child(
                        Button::new(("task-archive", id))
                            .small()
                            .ghost()
                            .label(label(if task.archived {
                                "workflow.restore"
                            } else {
                                "workflow.archive"
                            }))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_task(
                                    id,
                                    |t| {
                                        t.archived = !t.archived;
                                        Ok(())
                                    },
                                    cx,
                                )
                            })),
                    ),
            );
            page = page.child(card);
        }
        if tasks.len() > self.shown {
            page = page.child(
                Button::new("task-more")
                    .small()
                    .label(label("workflow.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shown += 30;
                        cx.notify();
                    })),
            );
        }
        page.into_any_element()
    }

    fn preset_page(&self, writable: bool, cx: &mut Context<Self>) -> AnyElement {
        let presets = self
            .host()
            .ok()
            .map(|host| {
                host.read(cx)
                    .store
                    .workflow
                    .presets
                    .iter()
                    .filter(|p| p.workspace == self.workspace && p.archived == self.show_archived)
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let configuration = self
            .host()
            .ok()
            .map(|host| {
                let app = host.read(cx);
                let s = self.source_session.and_then(|id| app.session(id));
                let selection = s
                    .map(ComposerSelection::from_session)
                    .unwrap_or_else(|| app.default_selection(app.new_session_provider));
                format!(
                    "{} · {} · {}",
                    selection.engine.label(),
                    selection
                        .model
                        .as_deref()
                        .unwrap_or(label("workflow.default_model")),
                    s.map_or(app.pending_permission, |s| s.permission).label()
                )
            })
            .unwrap_or_default();
        let mut page = v_flex()
            .gap_2()
            .child(label("workflow.preset_hint"))
            .child(configuration)
            .child(Input::new(&self.preset_name))
            .child(Textarea::new(&self.preset_prompt).h(px(100.)))
            .child(
                Button::new("preset-archived-filter")
                    .label(label("workflow.show_archived"))
                    .small()
                    .ghost()
                    .selected(self.show_archived)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_archived = !this.show_archived;
                        this.shown = 30;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("preset-save")
                    .small()
                    .label(label("workflow.save_preset"))
                    .disabled(!writable)
                    .on_click(cx.listener(|this, _, _, cx| this.save_preset(cx))),
            );
        for preset in presets.iter().take(self.shown) {
            let id = preset.id;
            page = page.child(
                v_flex()
                    .id(("project-preset", id))
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .rounded_md()
                    .child(format!(
                        "{} · {} · {} · {}",
                        preset.name,
                        preset.provider.label(),
                        preset
                            .model
                            .as_deref()
                            .unwrap_or(label("workflow.default_model")),
                        preset.permission.label()
                    ))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(preset.prompt.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(("preset-use", id))
                                    .small()
                                    .label(label("workflow.prepare_session"))
                                    .disabled(!writable)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.use_preset(id, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(("preset-archive", id))
                                    .small()
                                    .ghost()
                                    .label(label(if preset.archived {
                                        "workflow.restore"
                                    } else {
                                        "workflow.archive"
                                    }))
                                    .disabled(!writable)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let result = this.write(cx, |app, _| {
                                            let preset = app
                                                .store
                                                .workflow
                                                .presets
                                                .iter_mut()
                                                .find(|p| {
                                                    p.id == id && p.workspace == this.workspace
                                                })
                                                .ok_or_else(|| {
                                                    label("workflow.missing").to_owned()
                                                })?;
                                            preset.archived = !preset.archived;
                                            Ok(())
                                        });
                                        this.result(result, cx);
                                    })),
                            ),
                    ),
            );
        }
        if presets.len() > self.shown {
            page = page.child(
                Button::new("preset-more")
                    .small()
                    .label(label("workflow.more"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shown += 30;
                        cx.notify();
                    })),
            );
        }
        page.into_any_element()
    }

    fn handoff_page(&self, writable: bool, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        v_flex()
            .gap_2()
            .child(label("workflow.handoff_hint"))
            .child(format!(
                "{}: {}",
                label("workflow.source"),
                self.source_session
                    .map_or_else(|| "—".into(), |id| format!("#{id}"))
            ))
            .child(label("workflow.through"))
            .child(Input::new(&self.through))
            .child(self.native_branch_controls(writable, cx))
            .child(
                Button::new("handoff-prepare")
                    .small()
                    .label(label("workflow.prepare_context"))
                    .on_click(cx.listener(|this, _, window, cx| this.prepare_handoff(window, cx))),
            )
            .child(Textarea::new(&self.context).h(px(220.)))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("handoff-create")
                            .small()
                            .label(label("workflow.branch_session"))
                            .disabled(!writable)
                            .dropdown_menu(move |mut menu, _, _| {
                                for provider in Provider::ALL {
                                    let owner = owner.clone();
                                    menu =
                                        menu.item(PopupMenuItem::new(provider.label()).on_click(
                                            move |_, window, cx| {
                                                let _ = owner.update(cx, |this, cx| {
                                                    this.send_handoff(provider, window, cx)
                                                });
                                            },
                                        ));
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("handoff-copy")
                            .small()
                            .ghost()
                            .label(label("workflow.copy_markdown"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    this.context.read(cx).value().to_string(),
                                ));
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl Render for WorkflowPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self
            .host()
            .ok()
            .is_some_and(|host| host.read(cx).writable && !host.read(cx).closing);
        let mut body = v_flex()
            .id("project-workflow-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(180.)
            ))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(self.workspace.display().to_string()),
            );
        let mut tabs = h_flex().gap_1().flex_wrap();
        for (id, page, key) in [
            ("tasks", Page::Tasks, "workflow.tasks"),
            ("presets", Page::Presets, "workflow.presets"),
            ("handoff", Page::Handoff, "workflow.handoff"),
        ] {
            tabs = tabs.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .label(label(key))
                    .selected(self.page == page)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        this.error = None;
                        this.shown = 30;
                        cx.notify();
                    })),
            );
        }
        body = body.child(tabs);
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Ok(host) = self.host()
            && let Some(error) = &host.read(cx).persistence.error
        {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        body.child(match self.page {
            Page::Tasks => self.task_page(writable, cx),
            Page::Presets => self.preset_page(writable, cx),
            Page::Handoff => self.handoff_page(writable, cx),
        })
    }
}
