use super::*;

struct AutomationPanel {
    host: WeakEntity<ConduitApp>,
    workspace: PathBuf,
    preset: Option<u64>,
    hours: Entity<InputState>,
    event: bool,
    editing: Option<u64>,
    removing: Option<u64>,
    error: Option<String>,
    _subscription: Subscription,
}

impl ConduitApp {
    pub(in crate::app) fn automation_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Button::new("project-automations")
            .small()
            .compact()
            .ghost()
            .icon(FullIconName::Clock)
            .accessibility_label(label("automation.title"))
            .tooltip(label("automation.title"))
            .disabled(self.selected_workspace_path().is_none())
            .on_click(cx.listener(|this, _, window, cx| {
                let Some(workspace) = this.selected_workspace_path() else {
                    return;
                };
                let host = cx.entity();
                let panel = cx.new(|cx| AutomationPanel {
                    host: host.downgrade(),
                    workspace,
                    preset: None,
                    hours: cx.new(|cx| InputState::new(window, cx).default_value("24")),
                    event: false,
                    editing: None,
                    removing: None,
                    error: None,
                    _subscription: cx.observe(&host, |_, _, cx| cx.notify()),
                });
                window.open_dialog(cx, move |dialog, window, _| {
                    let panel = panel.clone();
                    dialog
                        .title(label("automation.title"))
                        .w(px(
                            (f32::from(window.viewport_size().width) - 48.).clamp(280., 950.)
                        ))
                        .content(move |content, _, _| content.child(panel.clone()))
                });
            }))
    }
}

impl AutomationPanel {
    fn save(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let interval = if self.event {
            None
        } else {
            let Some(seconds) = self
                .hours
                .read(cx)
                .value()
                .parse::<u64>()
                .ok()
                .and_then(|v| v.checked_mul(3600))
            else {
                self.error = Some(label("automation.limit").into());
                cx.notify();
                return;
            };
            Some(seconds)
        };
        let result = host.update(cx, |app, cx| {
            if !app.writable || app.closing {
                return Err(label("workflow.unavailable").to_owned());
            }
            let mut preset = if self.preset.is_none() && self.editing.is_some() {
                app.store
                    .scheduler
                    .schedules
                    .iter()
                    .find(|s| Some(s.id) == self.editing && s.preset.workspace == self.workspace)
                    .map(|s| s.preset.clone())
            } else {
                app.store
                    .workflow
                    .presets
                    .iter()
                    .find(|p| {
                        Some(p.id) == self.preset && p.workspace == self.workspace && !p.archived
                    })
                    .cloned()
            }
            .ok_or_else(|| label("automation.select_template").to_owned())?;
            preset.permission = PermissionLevel::ReadOnly;
            if let Some(id) = self.editing {
                app.store
                    .scheduler
                    .edit(id, &self.workspace, preset, interval, unix_now())?;
            } else {
                app.store.scheduler.add(preset, interval, unix_now())?;
            }
            app.persist();
            cx.notify();
            Ok(())
        });
        if result.is_ok() {
            self.editing = None;
            self.removing = None;
            self.preset = None;
        }
        self.error = result.err();
        cx.notify();
    }
    fn begin_edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let app = host.read(cx);
        let Some(schedule) = app
            .store
            .scheduler
            .schedules
            .iter()
            .find(|s| s.id == id && s.preset.workspace == self.workspace)
        else {
            return;
        };
        if !app.writable || app.closing || schedule.enabled || app.store.scheduler.busy(id) {
            self.error = Some(label("automation.pause_before_edit").into());
            cx.notify();
            return;
        }
        let (event, hours) = match schedule.trigger {
            Trigger::Interval { seconds, .. } => (false, (seconds / 3600).to_string()),
            Trigger::SessionCompleted => (true, "24".to_owned()),
        };
        self.event = event;
        self.hours
            .update(cx, |input, cx| input.set_value(hours, window, cx));
        self.preset = None;
        self.editing = Some(id);
        self.removing = None;
        self.error = None;
        cx.notify();
    }
    fn remove(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.removing != Some(id) {
            return;
        }
        let Some(host) = self.host.upgrade() else {
            return;
        };
        let result = host.update(cx, |app, cx| {
            if !app.writable || app.closing {
                return Err(label("workflow.unavailable").to_owned());
            }
            app.store.scheduler.remove(id, &self.workspace)?;
            app.persist();
            cx.notify();
            Ok(())
        });
        if result.is_ok() {
            self.removing = None;
            if self.editing == Some(id) {
                self.editing = None;
                self.preset = None;
            }
        }
        self.error = result.err();
        cx.notify();
    }
    fn toggle(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(host) = self.host.upgrade() {
            host.update(cx, |app, cx| {
                if !app.writable || app.closing {
                    return;
                }
                if let Some(schedule) = app
                    .store
                    .scheduler
                    .schedules
                    .iter_mut()
                    .find(|s| s.id == id && s.preset.workspace == self.workspace)
                {
                    schedule.enabled = !schedule.enabled;
                    if schedule.enabled
                        && let Trigger::Interval { seconds, next_at } = &mut schedule.trigger
                    {
                        *next_at = unix_now().saturating_add(*seconds);
                    }
                    app.persist();
                    cx.notify();
                }
            });
        }
    }
}

impl Render for AutomationPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let writable = app.writable && !app.closing;
        let presets: Vec<_> = app
            .store
            .workflow
            .presets
            .iter()
            .filter(|p| p.workspace == self.workspace && !p.archived)
            .map(|p| {
                (
                    p.id,
                    format!(
                        "{} · {} · {}",
                        p.name,
                        p.provider.label(),
                        p.model
                            .as_deref()
                            .unwrap_or(label("workflow.default_model"))
                    ),
                )
            })
            .collect();
        let selected = presets
            .iter()
            .find(|(id, _)| Some(*id) == self.preset)
            .map_or_else(
                || {
                    label(if self.editing.is_some() {
                        "automation.keep_template"
                    } else {
                        "automation.select_template"
                    })
                    .to_owned()
                },
                |(_, label)| label.clone(),
            );
        let owner = cx.weak_entity();
        let editing = self.editing.is_some();
        let mut body = v_flex()
            .id("automation-panel")
            .gap_2()
            .max_h(px(
                (f32::from(window.viewport_size().height) - 180.).max(180.)
            ))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .child(self.workspace.display().to_string()),
            )
            .child(label("automation.hint"))
            .child(
                Button::new("automation-template")
                    .small()
                    .label(selected)
                    .dropdown_menu(move |mut menu, _, _| {
                        if editing {
                            let owner = owner.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label("automation.keep_template")).on_click(
                                    move |_, _, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.preset = None;
                                            cx.notify();
                                        });
                                    },
                                ),
                            );
                        }
                        for (id, name) in &presets {
                            let id = *id;
                            let owner = owner.clone();
                            menu = menu.item(PopupMenuItem::new(name.clone()).on_click(
                                move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.preset = Some(id);
                                        cx.notify();
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
            .child(
                Button::new("automation-trigger")
                    .small()
                    .ghost()
                    .label(label(if self.event {
                        "automation.on_completion"
                    } else {
                        "automation.interval"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.event = !this.event;
                        cx.notify();
                    })),
            );
        if !self.event {
            body = body
                .child(label("automation.hours"))
                .child(Input::new(&self.hours));
        }
        if let Some(id) = self.editing {
            body = body
                .child(conduit::i18n::format(
                    "automation.editing",
                    &[("id", id.to_string())],
                ))
                .child(
                    Button::new("automation-cancel-edit")
                        .small()
                        .ghost()
                        .label(label("automation.cancel_edit"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.editing = None;
                            this.preset = None;
                            this.error = None;
                            cx.notify();
                        })),
                );
        }
        body = body.child(
            Button::new("automation-enable-new")
                .small()
                .label(label(if self.editing.is_some() {
                    "automation.save_edit"
                } else {
                    "automation.enable"
                }))
                .disabled(!writable || (self.preset.is_none() && self.editing.is_none()))
                .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
        );
        if let Some(error) = &self.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(error) = &app.persistence.error {
            body = body.child(
                div()
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        let schedules: Vec<_> = app
            .store
            .scheduler
            .schedules
            .iter()
            .filter(|s| s.preset.workspace == self.workspace)
            .collect();
        if !schedules.is_empty() {
            body = body.child(label("automation.pause_before_edit"));
        }
        for schedule in &schedules {
            let id = schedule.id;
            let editable = writable && !schedule.enabled && !app.store.scheduler.busy(id);
            let trigger = match schedule.trigger {
                Trigger::Interval { seconds, next_at } => conduit::i18n::format(
                    "automation.next",
                    &[
                        ("hours", (seconds / 3600).to_string()),
                        ("time", message_timestamp(next_at)),
                    ],
                ),
                Trigger::SessionCompleted => label("automation.on_completion").into(),
            };
            let mut row = v_flex()
                .gap_1()
                .p_2()
                .border_1()
                .border_color(rgb(palette().border))
                .rounded_md()
                .child(format!(
                    "{} · {} · {}",
                    schedule.preset.name,
                    schedule.preset.provider.label(),
                    label("domain.read_only")
                ))
                .child(trigger)
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new(("automation-toggle", id))
                                .small()
                                .ghost()
                                .label(label(if schedule.enabled {
                                    "automation.pause"
                                } else {
                                    "automation.resume"
                                }))
                                .disabled(!writable)
                                .on_click(cx.listener(move |this, _, _, cx| this.toggle(id, cx))),
                        )
                        .child(
                            Button::new(("automation-edit", id))
                                .small()
                                .ghost()
                                .label(label("automation.edit"))
                                .disabled(!editable)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.begin_edit(id, window, cx)
                                })),
                        )
                        .child(
                            Button::new(("automation-delete", id))
                                .small()
                                .ghost()
                                .label(label("automation.delete"))
                                .disabled(!editable)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.removing = Some(id);
                                    cx.notify();
                                })),
                        ),
                );
            if self.removing == Some(id) {
                row = row.child(label("automation.delete_hint")).child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new(("automation-confirm-delete", id))
                                .small()
                                .label(label("automation.confirm_delete"))
                                .disabled(!editable)
                                .on_click(cx.listener(move |this, _, _, cx| this.remove(id, cx))),
                        )
                        .child(
                            Button::new(("automation-cancel-delete", id))
                                .small()
                                .ghost()
                                .label(label("automation.cancel_delete"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.removing = None;
                                    cx.notify();
                                })),
                        ),
                );
            }
            body = body.child(row);
        }
        body = body.child(label("automation.history"));
        for run in app
            .store
            .scheduler
            .runs
            .iter()
            .rev()
            .filter(|r| {
                r.workspace.as_ref().map_or_else(
                    || schedules.iter().any(|s| s.id == r.schedule),
                    |workspace| workspace == &self.workspace,
                )
            })
            .take(50)
        {
            let mut row = v_flex()
                .p_2()
                .gap_1()
                .border_b_1()
                .border_color(rgb(palette().border))
                .child(format!(
                    "#{} · {} · {}",
                    run.schedule,
                    message_timestamp(run.at),
                    run.status.label()
                ));
            if !run.detail.is_empty() {
                row = row.child(div().text_size(ui_text_size(11.)).child(run.detail.clone()));
            }
            if let Some(id) = run.session {
                let host = host.downgrade();
                let workspace = self.workspace.clone();
                row = row.child(
                    Button::new(("automation-result", run.id))
                        .small()
                        .ghost()
                        .label(label("automation.open_session"))
                        .on_click(move |_, window, cx| {
                            let _ = host.update(cx, |app, cx| {
                                if let Some(index) = app.store.workspaces.iter().position(|w| {
                                    w.path == workspace && w.sessions.iter().any(|s| s.id == id)
                                }) {
                                    app.switch(index, Some(id), window, cx);
                                    window.close_dialog(cx);
                                    cx.notify();
                                }
                            });
                        }),
                );
            }
            body = body.child(row);
        }
        body.into_any_element()
    }
}
