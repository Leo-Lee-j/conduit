//! Explicit file snapshots, source context and bookmarked project outputs.
use super::*;
use crate::checkpoints::{Checkpoint, RestorePreview};

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

struct ProjectFilesPanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    path: Entity<InputState>,
    name: Entity<InputState>,
    range: Entity<InputState>,
    records: Vec<Checkpoint>,
    preview: Option<RestorePreview>,
    error: Option<String>,
    busy: bool,
    _task: Task<()>,
    _subscription: Subscription,
}

enum ResultData {
    List(Vec<Checkpoint>),
    Preview(RestorePreview),
    Context(String),
    Artifact { target: String, name: String },
}

impl ConduitApp {
    pub(super) fn project_files_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("project-files")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::Files)
            .tooltip(label("artifact.title"))
            .accessibility_label(label("artifact.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                let Some(workspace) = this.selected_workspace_path() else {
                    return;
                };
                let path = this
                    .current_file_key()
                    .filter(|key| key.workspace == workspace)
                    .map(|key| key.relative.display().to_string())
                    .unwrap_or_default();
                let host = cx.entity();
                let panel = cx.new(|cx| {
                    let mut panel = ProjectFilesPanel {
                        host: host.downgrade(),
                        workspace: workspace.clone(),
                        path: cx.new(|cx| {
                            InputState::new(window, cx)
                                .default_value(path)
                                .placeholder(label("artifact.target"))
                        }),
                        name: cx.new(|cx| {
                            InputState::new(window, cx).placeholder(label("workflow.name"))
                        }),
                        range: cx.new(|cx| {
                            InputState::new(window, cx).placeholder(label("artifact.lines"))
                        }),
                        records: vec![],
                        preview: None,
                        error: None,
                        busy: false,
                        _task: Task::ready(()),
                        _subscription: cx.observe(&host, |_, _, cx| cx.notify()),
                    };
                    panel.reload(window, cx);
                    panel
                });
                window.open_dialog(cx, move |dialog, window, _| {
                    let panel = panel.clone();
                    dialog
                        .title(label("artifact.title"))
                        .w(px(
                            (f32::from(window.viewport_size().width) - 48.).clamp(280., 1000.)
                        ))
                        .content(move |content, _, _| content.child(panel.clone()))
                });
            }))
    }
}

impl ProjectFilesPanel {
    fn writable(&self, cx: &App) -> bool {
        self.host.upgrade().is_some_and(|host| {
            let app = host.read(cx);
            app.writable
                && !app.closing
                && app
                    .store
                    .workspaces
                    .iter()
                    .any(|w| w.path == self.workspace)
        })
    }
    fn work(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce() -> anyhow::Result<ResultData> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        self._task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { f().map_err(|e| format!("{e:#}")) })
                .await;
            let _ = this.update_in(cx, move |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(ResultData::List(records)) => {
                        this.records = records;
                        this.preview = None;
                    }
                    Ok(ResultData::Preview(preview)) => this.preview = Some(preview),
                    Ok(ResultData::Artifact { target, name }) => {
                        if let Some(host) = this.host.upgrade() {
                            this.error = host
                                .update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return Err(label("workflow.unavailable").into());
                                    }
                                    app.store.workflow.save_artifact(
                                        crate::artifacts::Artifact {
                                            id: 0,
                                            workspace: this.workspace.clone(),
                                            title: name,
                                            target,
                                        },
                                    )?;
                                    app.persist();
                                    cx.notify();
                                    Ok::<_, String>(())
                                })
                                .err();
                        }
                    }
                    Ok(ResultData::Context(text)) => {
                        if let Some(host) = this.host.upgrade() {
                            this.error = host
                                .update(cx, |app, cx| {
                                    let target = (app.selected_workspace_path().as_ref()
                                        == Some(&this.workspace))
                                    .then_some(app.selected_session)
                                    .flatten();
                                    app.append_review_draft(
                                        &this.workspace,
                                        target,
                                        None,
                                        &text,
                                        window,
                                        cx,
                                    )
                                })
                                .err();
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        });
        cx.notify();
    }
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        self.work(window, cx, move || {
            crate::checkpoints::list(&crate::checkpoints::directory(), &workspace)
                .map(ResultData::List)
        });
    }
    fn capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable(cx) {
            return;
        }
        let workspace = self.workspace.clone();
        let path = PathBuf::from(self.path.read(cx).value().as_ref());
        let name = self.name.read(cx).value().to_string();
        self.work(window, cx, move || {
            let directory = crate::checkpoints::directory();
            crate::checkpoints::capture(&directory, &workspace, &path, &name)?;
            crate::checkpoints::list(&directory, &workspace).map(ResultData::List)
        });
    }
    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable(cx) {
            return;
        }
        let blocked = self.host.upgrade().is_none_or(|host| {
            let app = host.read(cx);
            app.worktree_has_runs(&self.workspace)
                || app.terminals.workspace_busy(&self.workspace)
                || app.git.operation.is_some()
        });
        if blocked {
            self.error = Some(label("checkpoint.busy").into());
            cx.notify();
            return;
        }
        let Some(preview) = self.preview.clone() else {
            return;
        };
        let workspace = self.workspace.clone();
        self.work(window, cx, move || {
            let directory = crate::checkpoints::directory();
            crate::checkpoints::restore(&directory, &preview)?;
            crate::checkpoints::list(&directory, &workspace).map(ResultData::List)
        });
    }
}

impl Render for ProjectFilesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writable = self.writable(cx) && !self.busy;
        let mut body = v_flex()
            .id("project-files-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(160.)
            ))
            .overflow_y_scroll()
            .child(self.workspace.display().to_string())
            .child(label("artifact.hint"))
            .child(Input::new(&self.path))
            .child(Input::new(&self.name))
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("checkpoint-capture")
                            .small()
                            .label(label("checkpoint.capture"))
                            .disabled(!writable)
                            .on_click(cx.listener(|this, _, window, cx| this.capture(window, cx))),
                    )
                    .child(
                        Button::new("artifact-save")
                            .small()
                            .label(label("artifact.save"))
                            .disabled(!writable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if !this.writable(cx) {
                                    return;
                                }
                                let workspace = this.workspace.clone();
                                let target = this.path.read(cx).value().to_string();
                                let name = this.name.read(cx).value().to_string();
                                this.work(window, cx, move || {
                                    crate::artifacts::url(&workspace, &target)?;
                                    Ok(ResultData::Artifact { target, name })
                                });
                            })),
                    )
                    .child(
                        Button::new("checkpoint-reload")
                            .small()
                            .ghost()
                            .label(label("github.refresh"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.reload(window, cx))),
                    ),
            )
            .child(Input::new(&self.range))
            .child(
                Button::new("file-context")
                    .small()
                    .label(label("artifact.attach"))
                    .disabled(!writable)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let workspace = this.workspace.clone();
                        let path = PathBuf::from(this.path.read(cx).value().as_ref());
                        let range = this.range.read(cx).value().to_string();
                        this.work(window, cx, move || {
                            crate::artifacts::context(&workspace, &path, &range)
                                .map(ResultData::Context)
                        });
                    })),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if self.busy {
            body = body.child(label("github.loading"));
        }
        if let Some(host) = self.host.upgrade() {
            let artifacts: Vec<_> = host
                .read(cx)
                .store
                .workflow
                .artifacts
                .iter()
                .filter(|a| a.workspace == self.workspace)
                .cloned()
                .collect();
            for artifact in artifacts {
                let id = artifact.id;
                let target = artifact.target.clone();
                let workspace = artifact.workspace.clone();
                body = body.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new(("artifact-open", id))
                                .small()
                                .ghost()
                                .label(artifact.title)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match crate::artifacts::url(&workspace, &target) {
                                        Ok(url) => cx.open_url(&url),
                                        Err(e) => this.error = Some(format!("{e:#}")),
                                    };
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("artifact-remove", id))
                                .small()
                                .ghost()
                                .label(label("artifact.remove"))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(host) = this.host.upgrade() {
                                        host.update(cx, |app, cx| {
                                            if app.writable && !app.closing {
                                                app.store.workflow.artifacts.retain(|a| {
                                                    a.id != id || a.workspace != this.workspace
                                                });
                                                app.persist();
                                                cx.notify();
                                            }
                                        });
                                    }
                                })),
                        ),
                );
            }
        }
        body = body.child(label("checkpoint.hint"));
        for record in &self.records {
            let record = record.clone();
            body = body.child(
                Button::new(SharedString::from(format!("checkpoint-{}", record.id)))
                    .small()
                    .ghost()
                    .label(format!(
                        "{} · {} · {}",
                        record.title,
                        record.relative.display(),
                        message_timestamp(record.at)
                    ))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let record = record.clone();
                        this.work(window, cx, move || {
                            crate::checkpoints::preview(record).map(ResultData::Preview)
                        });
                    })),
            );
        }
        if let Some(preview) = &self.preview {
            body = body
                .child(format!(
                    "{} · {}",
                    label("checkpoint.preview"),
                    preview.checkpoint.relative.display()
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(label("checkpoint.current"))
                                .child(
                                    div().child(
                                        preview
                                            .current
                                            .clone()
                                            .unwrap_or_else(|| label("checkpoint.missing").into()),
                                    ),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(label("checkpoint.saved"))
                                .child(div().child(preview.checkpoint.text.clone())),
                        ),
                )
                .child(
                    Button::new("checkpoint-restore")
                        .small()
                        .label(label("checkpoint.restore"))
                        .disabled(!writable)
                        .on_click(cx.listener(|this, _, window, cx| this.restore(window, cx))),
                );
        }
        body.into_any_element()
    }
}
