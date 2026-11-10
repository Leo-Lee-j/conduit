use super::*;
use crate::environments::{Environment, Target};
fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}
struct EnvironmentsPanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    name: Entity<InputState>,
    endpoint: Entity<InputState>,
    directory: Entity<InputState>,
    ssh: bool,
    error: Option<String>,
    _subscription: Subscription,
}

impl ConduitApp {
    pub(super) fn environments_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("project-environments")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::Globe)
            .tooltip(label("environment.title"))
            .accessibility_label(label("environment.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                let Some(workspace) = this.selected_workspace_path() else {
                    return;
                };
                let host = cx.entity();
                let panel = cx.new(|cx| EnvironmentsPanel {
                    host: host.downgrade(),
                    workspace,
                    name: cx
                        .new(|cx| InputState::new(window, cx).placeholder(label("workflow.name"))),
                    endpoint: cx.new(|cx| {
                        InputState::new(window, cx).placeholder(label("environment.host"))
                    }),
                    directory: cx.new(|cx| {
                        InputState::new(window, cx)
                            .default_value("/tmp")
                            .placeholder(label("environment.directory"))
                    }),
                    ssh: false,
                    error: None,
                    _subscription: cx.observe(&host, |_, _, cx| cx.notify()),
                });
                window.open_dialog(cx, move |dialog, window, _| {
                    let panel = panel.clone();
                    dialog
                        .title(label("environment.title"))
                        .w(px(
                            (f32::from(window.viewport_size().width) - 48.).clamp(280., 800.)
                        ))
                        .content(move |body, _, _| body.child(panel.clone()))
                });
            }))
    }
}
impl Render for EnvironmentsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let environments: Vec<_> = app
            .store
            .workflow
            .environments
            .iter()
            .filter(|e| e.workspace == self.workspace)
            .cloned()
            .collect();
        let mut body = v_flex()
            .id("environment-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(160.)
            ))
            .overflow_y_scroll()
            .child(label("environment.scope"))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("environment-wsl")
                            .small()
                            .label("WSL")
                            .selected(!self.ssh)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ssh = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("environment-ssh")
                            .small()
                            .label("SSH")
                            .selected(self.ssh)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ssh = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(Input::new(&self.name))
            .child(Input::new(&self.endpoint))
            .child(Input::new(&self.directory))
            .child(
                Button::new("environment-save")
                    .small()
                    .label(label("environment.save"))
                    .disabled(!writable)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let environment = Environment {
                            id: 0,
                            workspace: this.workspace.clone(),
                            name: this.name.read(cx).value().to_string(),
                            target: if this.ssh { Target::Ssh } else { Target::Wsl },
                            host: this.endpoint.read(cx).value().to_string(),
                            directory: this.directory.read(cx).value().to_string(),
                        };
                        if let Some(host) = this.host.upgrade() {
                            this.error = host
                                .update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return Err(label("workflow.unavailable").into());
                                    }
                                    app.store.workflow.save_environment(environment)?;
                                    app.persist();
                                    cx.notify();
                                    Ok::<_, String>(())
                                })
                                .err();
                        }
                        cx.notify();
                    })),
            );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        for environment in environments {
            let id = environment.id;
            let weak = self.host.clone();
            body = body.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(div().child(format!(
                        "{} · {}:{}",
                        environment.name, environment.host, environment.directory
                    )))
                    .child(
                        Button::new(("environment-connect", id))
                            .small()
                            .label(label("environment.connect"))
                            .disabled(!writable)
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |app, cx| {
                                    app.environment_terminal(environment.clone(), window, cx);
                                });
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new(("environment-remove", id))
                            .small()
                            .ghost()
                            .label(label("environment.remove"))
                            .disabled(!writable)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(host) = this.host.upgrade() {
                                    host.update(cx, |app, cx| {
                                        if app.writable && !app.closing {
                                            app.store.workflow.environments.retain(|e| {
                                                e.id != id || e.workspace != this.workspace
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
        body.into_any_element()
    }
}
