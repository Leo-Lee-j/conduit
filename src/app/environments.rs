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
    port: Entity<InputState>,
    target: Target,
    editing: Option<u64>,
    form: bool,
    baseline: [String; 4],
    confirm_discard: bool,
    removing: Option<u64>,
    error: Option<String>,
    detecting: bool,
    distributions: Option<Vec<String>>,
    discovery_error: Option<String>,
    _task: Task<()>,
    _subscription: Subscription,
}

impl ConduitApp {
    pub(super) fn environments_button(
        &self,
        cx: &mut Context<Self>,
        popover: Option<WeakEntity<gpui_kit::component::popover::PopoverState>>,
    ) -> Button {
        Button::new("project-environments")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::Globe)
            .tooltip(label("environment.title"))
            .accessibility_label(label("environment.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(popover) = &popover {
                    let _ = popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                }
                this.open_environments(window, cx);
            }))
    }
    fn open_environments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        let host = cx.entity();
        let panel = cx.new(|cx| EnvironmentsPanel::new(&host, workspace, window, cx));
        window.open_dialog(cx, move |dialog, window, _| {
            let dismiss = panel.clone();
            let panel = panel.clone();
            dialog
                .title(label("environment.title"))
                .overlay_closable(false)
                .on_cancel(move |_, _, cx| dismiss.update(cx, |panel, cx| panel.can_close(cx)))
                .w(px(
                    (f32::from(window.viewport_size().width) - 48.).clamp(280., 680.)
                ))
                .content(move |body, _, _| body.child(panel.clone()))
        });
    }
}
impl EnvironmentsPanel {
    fn new(
        host: &Entity<ConduitApp>,
        workspace: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            host: host.downgrade(),
            workspace,
            name: cx.new(|cx| InputState::new(window, cx)),
            endpoint: cx.new(|cx| InputState::new(window, cx)),
            directory: cx.new(|cx| {
                InputState::new(window, cx).placeholder(label("environment.directory_placeholder"))
            }),
            port: cx.new(|cx| {
                InputState::new(window, cx).placeholder(label("environment.port_placeholder"))
            }),
            target: Target::Wsl,
            editing: None,
            form: false,
            baseline: Default::default(),
            confirm_discard: false,
            removing: None,
            error: None,
            detecting: false,
            distributions: None,
            discovery_error: None,
            _task: Task::ready(()),
            _subscription: cx.observe(host, |_, _, cx| cx.notify()),
        }
    }

    fn values(&self, cx: &App) -> [String; 4] {
        [&self.name, &self.endpoint, &self.directory, &self.port]
            .map(|input| input.read(cx).value().to_string())
    }
    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.form && self.values(cx) != self.baseline && !self.confirm_discard {
            self.confirm_discard = true;
            cx.notify();
            return false;
        }
        true
    }
    fn writable(&self, cx: &App) -> bool {
        self.host.upgrade().is_some_and(|host| {
            let app = host.read(cx);
            app.writable
                && !app.closing
                && !app.updater.preparing()
                && app.selected_workspace_path().as_ref() == Some(&self.workspace)
        })
    }
    fn edit(
        &mut self,
        environment: Option<Environment>,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.target = target;
        self.editing = environment.as_ref().map(|e| e.id);
        let values = environment.map_or_else(
            || [String::new(), String::new(), String::new(), String::new()],
            |e| {
                [
                    e.name,
                    e.host,
                    e.directory,
                    e.port.map_or(String::new(), |p| p.to_string()),
                ]
            },
        );
        for (input, value) in [&self.name, &self.endpoint, &self.directory, &self.port]
            .into_iter()
            .zip(values.iter())
        {
            input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
        }
        self.baseline = values;
        self.form = true;
        self.confirm_discard = false;
        self.removing = None;
        self.error = None;
        self.endpoint.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn draft(&self, cx: &App) -> Result<Environment, String> {
        let [name, host, directory, port] = self.values(cx);
        let host = host.trim().to_owned();
        if host.is_empty() {
            return Err(label(if self.target == Target::Wsl {
                "environment.invalid_wsl"
            } else {
                "environment.invalid_ssh"
            })
            .into());
        }
        let port = if self.target == Target::Wsl || port.trim().is_empty() {
            None
        } else {
            Some(
                port.trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .ok_or_else(|| label("environment.invalid_port").to_string())?,
            )
        };
        let environment = Environment {
            id: self.editing.unwrap_or(0),
            workspace: self.workspace.clone(),
            name: if name.trim().is_empty() {
                host.clone()
            } else {
                name.trim().to_owned()
            },
            target: self.target,
            host,
            directory,
            port,
        };
        environment.validate().map_err(|e| e.to_string())?;
        Ok(environment)
    }
    fn save(&mut self, connect: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        if !self.writable(cx) {
            self.error = Some(label("workflow.unavailable").into());
            cx.notify();
            return;
        }
        let mut environment = match self.draft(cx) {
            Ok(environment) => environment,
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
            let previous = app.store.workflow.clone();
            let id = app.store.workflow.save_environment(environment.clone())?;
            if !app.persist() {
                app.store.workflow = previous;
                return Err(label("environment.save_failed").to_string());
            }
            cx.notify();
            Ok(id)
        });
        match result {
            Ok(id) => {
                environment.id = id;
                self.editing = Some(id);
                self.baseline = self.values(cx);
                self.confirm_discard = false;
                if connect {
                    self.connect(environment, window, cx);
                } else {
                    self.form = false;
                }
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
    fn connect(&mut self, environment: Environment, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable(cx) {
            self.error = Some(label("workflow.unavailable").into());
            cx.notify();
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        self.error = host
            .update(cx, |app, cx| {
                app.environment_terminal(environment, window, cx)
            })
            .err();
        cx.notify();
    }
    fn detect(&mut self, cx: &mut Context<Self>) {
        if self.detecting {
            return;
        }
        self.detecting = true;
        self.discovery_error = None;
        self._task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async { crate::environments::distributions() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.detecting = false;
                match result {
                    Ok(names) => this.distributions = Some(names),
                    Err(error) => this.discovery_error = Some(error.to_string()),
                }
                cx.notify();
            });
        });
        cx.notify();
    }
    fn field(
        &self,
        title: &'static str,
        input: &Entity<InputState>,
        help: &'static str,
        enabled: bool,
    ) -> impl IntoElement {
        v_flex()
            .gap_1()
            .w_full()
            .child(div().text_size(ui_text_size(13.)).child(label(title)))
            .child(
                Input::new(input)
                    .aria_label(label(title))
                    .disabled(!enabled),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(label(help)),
            )
    }
}
impl Render for EnvironmentsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = self.writable(cx);
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
            .gap_4()
            .min_h_0()
            .flex_1()
            .overflow_y_scroll()
            .text_size(ui_text_size(13.))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .child(label("environment.intro")),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(crate::workspace_files::display_path(&self.workspace)),
                    ),
            );
        if !writable {
            body = body.child(
                div()
                    .text_color(rgb(palette().warning_text))
                    .child(label("environment.readonly")),
            );
        }
        if self.form {
            body = body.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(format!(
                        "{} · {}",
                        label(if self.editing.is_some() {
                            "environment.edit"
                        } else {
                            "environment.new"
                        }),
                        self.target.label()
                    )))
                    .child(
                        Button::new("environment-back")
                            .small()
                            .ghost()
                            .label(label(if self.confirm_discard {
                                "environment.discard"
                            } else {
                                "environment.back"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.can_close(cx) {
                                    this.form = false;
                                    this.confirm_discard = false;
                                    this.error = None;
                                }
                                cx.notify();
                            })),
                    ),
            );
            let ssh = self.target == Target::Ssh;
            body = body.child(self.field(
                if ssh {
                    "environment.ssh_host"
                } else {
                    "environment.wsl_host"
                },
                &self.endpoint,
                if ssh {
                    "environment.ssh_help"
                } else {
                    "environment.wsl_help"
                },
                writable,
            ));
            if !ssh {
                let mut discovery = v_flex().gap_2().child(
                    Button::new("environment-detect")
                        .small()
                        .ghost()
                        .label(label(if self.detecting {
                            "environment.detecting"
                        } else {
                            "environment.detect"
                        }))
                        .disabled(self.detecting || !writable)
                        .on_click(cx.listener(|this, _, _, cx| this.detect(cx))),
                );
                if let Some(error) = &self.discovery_error {
                    discovery = discovery.child(
                        div()
                            .text_color(rgb(palette().warning_text))
                            .child(error.clone()),
                    );
                }
                if let Some(names) = &self.distributions {
                    if names.is_empty() {
                        discovery = discovery.child(
                            div()
                                .text_color(rgb(palette().muted))
                                .child(label("environment.no_distributions")),
                        );
                    } else {
                        let mut choices = h_flex().gap_2().flex_wrap();
                        for (index, name) in names.iter().enumerate() {
                            let name = name.clone();
                            choices = choices.child(
                                Button::new(("environment-distribution", index))
                                    .small()
                                    .label(name.clone())
                                    .disabled(!writable)
                                    .selected(self.endpoint.read(cx).value().as_ref() == name)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.endpoint.update(cx, |input, cx| {
                                            input.set_value(name.clone(), window, cx)
                                        });
                                        cx.notify();
                                    })),
                            );
                        }
                        discovery = discovery.child(choices);
                    }
                }
                body = body.child(discovery);
            } else {
                body = body.child(self.field(
                    "environment.port",
                    &self.port,
                    "environment.port_help",
                    writable,
                ));
            }
            body = body
                .child(self.field(
                    "environment.directory",
                    &self.directory,
                    "environment.directory_help",
                    writable,
                ))
                .child(self.field(
                    "environment.name",
                    &self.name,
                    "environment.name_help",
                    writable,
                ));
            if ssh {
                body = body.child(
                    div()
                        .text_color(rgb(palette().muted))
                        .text_size(ui_text_size(12.))
                        .child(label("environment.authentication")),
                );
            }
        } else {
            body = body.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("environment-add-wsl")
                            .small()
                            .label(label("environment.add_wsl"))
                            .disabled(!writable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit(None, Target::Wsl, window, cx)
                            })),
                    )
                    .child(
                        Button::new("environment-add-ssh")
                            .small()
                            .label(label("environment.add_ssh"))
                            .disabled(!writable)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit(None, Target::Ssh, window, cx)
                            })),
                    ),
            );
            if environments.is_empty() {
                body = body.child(
                    v_flex()
                        .gap_2()
                        .p_4()
                        .rounded_md()
                        .bg(rgb(palette().surface))
                        .child(label("environment.empty"))
                        .child(
                            div()
                                .text_color(rgb(palette().muted))
                                .child(label("environment.empty_help")),
                        ),
                );
            }
            for environment in environments {
                let id = environment.id;
                let edit = environment.clone();
                let connect = environment.clone();
                let deleting = self.removing == Some(id);
                let mut row = v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(palette().border))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .child(environment.name.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(palette().muted))
                                    .text_size(ui_text_size(12.))
                                    .child(environment.target.label()),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .truncate()
                            .child(environment.destination()),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .truncate()
                            .child(
                                if environment.directory.is_empty() || environment.directory == "~"
                                {
                                    label("environment.home").to_string()
                                } else {
                                    environment.directory.clone()
                                },
                            ),
                    );
                if deleting {
                    row = row.child(
                        div()
                            .text_color(rgb(palette().warning_text))
                            .child(label("environment.remove_help")),
                    );
                }
                row = row.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new(("environment-connect", id))
                                .small()
                                .label(label("environment.connect"))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.connect(connect.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new(("environment-edit", id))
                                .small()
                                .ghost()
                                .label(label("environment.edit"))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit(Some(edit.clone()), edit.target, window, cx)
                                })),
                        )
                        .child(
                            Button::new(("environment-remove", id))
                                .small()
                                .ghost()
                                .label(label(if deleting {
                                    "environment.confirm_remove"
                                } else {
                                    "environment.remove"
                                }))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.writable(cx) {
                                        return;
                                    }
                                    if this.removing != Some(id) {
                                        this.removing = Some(id);
                                        cx.notify();
                                        return;
                                    }
                                    if let Some(host) = this.host.upgrade() {
                                        this.error = host.update(cx, |app, cx| {
                                            let previous = app.store.workflow.clone();
                                            app.store.workflow.environments.retain(|e| {
                                                e.id != id || e.workspace != this.workspace
                                            });
                                            let error = if app.persist() {
                                                None
                                            } else {
                                                app.store.workflow = previous;
                                                Some(label("environment.save_failed").to_string())
                                            };
                                            cx.notify();
                                            error
                                        });
                                    }
                                    this.removing = None;
                                    cx.notify();
                                })),
                        )
                        .when(deleting, |actions| {
                            actions.child(
                                Button::new(("environment-cancel-remove", id))
                                    .small()
                                    .ghost()
                                    .label(label("environment.cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.removing = None;
                                        cx.notify();
                                    })),
                            )
                        }),
                );
                body = body.child(row);
            }
        }
        v_flex()
            .gap_3()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 176.).max(120.)
            ))
            .child(body)
            .when(self.confirm_discard, |panel| {
                panel.child(
                    div()
                        .flex_shrink_0()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().warning_text))
                        .child(label("environment.unsaved")),
                )
            })
            .when_some(self.error.clone(), |panel, error| {
                panel.child(
                    div()
                        .flex_shrink_0()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            })
            .when(self.form, |panel| {
                panel.child(
                    h_flex()
                        .gap_2()
                        .flex_shrink_0()
                        .flex_wrap()
                        .child(
                            Button::new("environment-save-connect")
                                .primary()
                                .label(label("environment.save_connect"))
                                .disabled(!writable)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save(true, window, cx)),
                                ),
                        )
                        .child(
                            Button::new("environment-save")
                                .label(label("environment.save"))
                                .disabled(!writable)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .pt_3()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(label("environment.scope")),
            )
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    #[gpui_kit::test]
    fn edits_validate_before_saving_and_reject_stale_workspace_actions(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, workspace) = crate::app::queue::tests::fixture(cx, "environment-edit");
        cx.update(|window, cx| {
            app.update(cx, |app, _| {
                app.startup = None;
                app.writable = true;
            });
            let panel = cx.new(|cx| EnvironmentsPanel::new(&app, workspace, window, cx));
            panel.update(cx, |panel, cx| {
                panel.edit(None, Target::Ssh, window, cx);
                panel
                    .endpoint
                    .update(cx, |input, cx| input.set_value("dev@host", window, cx));
                panel
                    .port
                    .update(cx, |input, cx| input.set_value("70000", window, cx));
                panel.save(false, window, cx);
                assert!(panel.form && panel.error.is_some());
                assert!(app.read(cx).store.workflow.environments.is_empty());
                assert!(!panel.can_close(cx), "unsaved form must be guarded");
                panel
                    .port
                    .update(cx, |input, cx| input.set_value("2222", window, cx));
                panel.save(false, window, cx);
                assert!(panel.error.is_none(), "{:?}", panel.error);
                assert!(!panel.form);
                let saved = app.read(cx).store.workflow.environments[0].clone();
                assert_eq!(saved.name, "dev@host");
                assert_eq!(saved.port, Some(2222));
                panel.edit(Some(saved.clone()), Target::Ssh, window, cx);
                panel
                    .name
                    .update(cx, |input, cx| input.set_value("Development", window, cx));
                panel.save(false, window, cx);
                assert!(panel.error.is_none());
                let environments = &app.read(cx).store.workflow.environments;
                assert_eq!(environments.len(), 1);
                assert_eq!(environments[0].id, saved.id);
                assert_eq!(environments[0].name, "Development");
                panel.edit(Some(saved), Target::Ssh, window, cx);
                app.update(cx, |app, _| app.selected_workspace = Some(1));
                panel.save(false, window, cx);
                assert!(panel.error.is_some());
                assert_eq!(
                    app.read(cx).store.workflow.environments[0].name,
                    "Development"
                );
                app.update(cx, |app, _| {
                    app.selected_workspace = Some(0);
                    app.writable = false;
                });
                panel.save(false, window, cx);
                assert!(panel.error.is_some());
                assert_eq!(
                    app.read(cx).store.workflow.environments[0].name,
                    "Development"
                );
            });
        });
    }
}
