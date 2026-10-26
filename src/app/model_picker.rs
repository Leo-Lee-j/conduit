//! Provider, permission, and model selection controls.

use super::*;

// Keep notifications local. Observing the parent ConduitApp here creates a
// parent/child invalidation cycle while the popover is open.
struct SessionModelPicker {
    app: Entity<ConduitApp>,
    session_id: u64,
    search: Entity<InputState>,
    slider: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl SessionModelPicker {
    fn new(
        app: Entity<ConduitApp>,
        session_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let effort = app
            .read(cx)
            .session(session_id)
            .map(|s| s.reasoning_effort)
            .unwrap_or_default();
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(4.)
                .step(1.)
                .default_value(effort.index() as f32)
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("搜索模型名称或 ID"));
        let view = cx.entity();
        let subscriptions = vec![
            cx.subscribe(
                &app,
                |_, _, _: &crate::providers::ProviderCatalogChanged, cx| cx.notify(),
            ),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            window.subscribe(&slider, cx, move |_, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    let effort = ReasoningEffort::ALL[(value.start().round() as usize).min(4)];
                    view.update(cx, |this, cx| {
                        // SliderState snaps the emitted value to the configured
                        // step, but keeps the pointer percentage while dragging.
                        // Write the snapped value back so the thumb also moves in
                        // the five discrete reasoning levels.
                        this.slider.update(cx, |slider, cx| {
                            slider.set_value(effort.index() as f32, window, cx)
                        });
                        this.app.update(cx, |app, cx| {
                            if !app.writable
                                || app.closing
                                || app.runs.contains_key(&this.session_id)
                            {
                                return;
                            }
                            if let Some(session) = app.session_mut(this.session_id) {
                                session.reasoning_effort = effort;
                            }
                            app.dirty = true;
                            cx.notify();
                        });
                        cx.notify();
                    });
                }
            }),
        ];
        Self {
            app,
            session_id,
            search,
            slider,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for SessionModelPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.read(cx);
        let Some(session) = app.session(self.session_id) else {
            return v_flex();
        };
        let engine = session.provider;
        let channel = session.channel.clone();
        let selected = session.model.clone();
        let effort = session.reasoning_effort;
        let channel_label = app.channel_label(engine, &channel);
        let (note, mut models) = app.channel_models(engine, &channel);
        let disabled = !app.writable || app.closing || app.runs.contains_key(&self.session_id);
        let mut channels = vec![(ModelChannel::Cli, "CLI 默认".to_string())];
        channels.extend(
            app.store
                .providers
                .iter()
                .filter(|p| p.engine == engine)
                .map(|p| (ModelChannel::Provider(p.id.clone()), p.name.clone())),
        );
        if let Some(id) = &selected
            && !models.iter().any(|m| m.id == *id)
        {
            models::push_model(&mut models, id, id, "当前会话模型");
        }
        let query = self.search.read(cx).value().trim().to_lowercase();
        models.retain(|m| {
            query.is_empty()
                || m.id.to_lowercase().contains(&query)
                || m.label.to_lowercase().contains(&query)
        });
        let count = models.len();
        let picker = cx.entity();
        let channel_picker = Popover::new("session-channel-picker")
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("session-channel-trigger")
                    .ghost()
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .disabled(disabled)
                    .accessibility_label("选择 Provider")
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .debug_selector(|| "session-channel-control".into())
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().faint))
                                    .child("Provider"),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette().text))
                                    .child(channel_label),
                            )
                            .child(Icon::new(IconName::ChevronDown).small()),
                    ),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("session-channel-list")
                    .w(px(300.))
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .gap_1();
                for (index, (option, label)) in channels.iter().enumerate() {
                    let active = *option == channel;
                    let option = option.clone();
                    let picker = picker.clone();
                    let popover = popover.clone();
                    list = list.child(
                        Button::new(("session-channel", index))
                            .ghost()
                            .w_full()
                            .h(px(34.))
                            .px_2()
                            .cursor_pointer()
                            .when(active, |b| b.bg(rgb(palette().surface_hover)))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("session-provider-option-{index}")
                                            })
                                            .flex_1()
                                            .truncate()
                                            .child(label.clone()),
                                    )
                                    .when(active, |r| r.child(Icon::new(IconName::Check).small())),
                            )
                            .on_click(move |_, window, cx| {
                                picker.update(cx, |this, cx| {
                                    this.app.update(cx, |app, cx| {
                                        if !app.writable
                                            || app.closing
                                            || app.runs.contains_key(&this.session_id)
                                        {
                                            return;
                                        }
                                        let model = None;
                                        if let Some(s) = app.session_mut(this.session_id) {
                                            if s.channel == option {
                                                return;
                                            }
                                            s.channel = option.clone();
                                            s.model = model;
                                            s.reasoning_effort = ReasoningEffort::Default;
                                        }
                                        app.persist();
                                        cx.notify();
                                    });
                                    this.search.update(cx, |s, cx| s.set_value("", window, cx));
                                    let effort = this
                                        .app
                                        .read(cx)
                                        .session(this.session_id)
                                        .map(|s| s.reasoning_effort)
                                        .unwrap_or_default();
                                    this.slider.update(cx, |s, cx| {
                                        s.set_value(effort.index() as f32, window, cx)
                                    });
                                    cx.notify();
                                });
                                if let Some(p) = popover.upgrade() {
                                    p.update(cx, |p, cx| p.dismiss(window, cx));
                                }
                            }),
                    );
                }
                list
            });
        let mut list = v_flex()
            .id("session-model-options")
            .debug_selector(|| "session-model-list".into())
            .max_h((window.viewport_size().height - px(290.)).clamp(px(80.), px(250.)))
            .overflow_y_scroll()
            .gap_1();
        let options = std::iter::once((
            None,
            "默认模型".to_string(),
            "跟随所选 Provider 的默认配置".to_string(),
        ))
        .chain(models.into_iter().map(|m| {
            (
                Some(m.id.clone()),
                m.label.clone(),
                if m.note.is_empty() && m.label != m.id {
                    m.id
                } else {
                    m.note
                },
            )
        }));
        for (index, (model, label, note)) in options.enumerate() {
            let active = selected == model;
            let mark = model_mark(engine, model.as_deref());
            list = list.child(
                Button::new(("session-model-option", index))
                    .ghost()
                    .w_full()
                    .h(px(46.))
                    .px_2()
                    .rounded(px(7.))
                    .disabled(disabled)
                    .cursor_pointer()
                    .accessibility_label(format!("模型 {label}"))
                    .when(active, |b| b.bg(rgb(palette().surface_hover)))
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(mark)
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(12.))
                                            .text_color(rgb(palette().text))
                                            .child(label),
                                    )
                                    .when(!note.is_empty(), |c| {
                                        c.child(
                                            div()
                                                .truncate()
                                                .text_size(px(10.))
                                                .text_color(rgb(palette().faint))
                                                .child(note),
                                        )
                                    }),
                            )
                            .when(active, |r| r.child(Icon::new(IconName::Check).small())),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.app.update(cx, |app, cx| {
                            if !app.runs.contains_key(&this.session_id) {
                                app.set_session_model(this.session_id, model.clone(), cx);
                            }
                        });
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .w(px(340.))
            .gap_2()
            .child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("模型与思考"),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(engine.label()),
                    ),
            )
            .child(channel_picker)
            .child(
                div()
                    .debug_selector(|| "session-model-search".into())
                    .child(Input::new(&self.search).h(px(30.))),
            )
            .child(list)
            .when(count == 0, |c| {
                c.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("没有匹配的模型"),
                )
            })
            .when_some(note, |c, note| {
                c.child(
                    div()
                        .px_2()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(note),
                )
            })
            .child(
                v_flex()
                    .px_2()
                    .pt_3()
                    .pb_2()
                    .gap_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        h_flex()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child("思考强度"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child(effort.label()),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "session-effort-control".into())
                            .w_full()
                            .child(
                                Slider::new(&self.slider)
                                    .w_full()
                                    .disabled(disabled)
                                    .accessibility_label("模型思考强度"),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .children(ReasoningEffort::ALL.map(|level| {
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(level.label())
                            })),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .line_height(relative(1.4))
                            .text_color(rgb(palette().faint))
                            .child("更高强度可能增加耗时；支持的档位由模型决定"),
                    ),
            )
            .child(
                h_flex()
                    .debug_selector(|| "session-model-footer".into())
                    .px_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child("仅影响本会话，下一条消息生效"),
                    )
                    .child(
                        Button::new("session-model-refresh")
                            .ghost()
                            .small()
                            .label("刷新")
                            .cursor_pointer()
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.app
                                    .update(cx, |app, cx| app.reload_all_models_async(window, cx));
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl ConduitApp {
    pub(super) fn engine_picker(&self, view: Entity<Self>) -> AnyElement {
        let provider = self.new_session_provider;
        let trigger = Button::new("engine-picker-trigger")
            .ghost()
            .small()
            .h(px(22.))
            .px_1()
            .cursor_pointer()
            .tooltip("切换引擎")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(provider.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("engine-picker")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(180.)).gap(px(1.));
                for option in [Provider::Claude, Provider::Codex] {
                    let app = view.clone();
                    let popover = popover.clone();
                    let active = option == provider;
                    list = list.child(
                        Button::new(if option == Provider::Claude {
                            "engine-option-claude"
                        } else {
                            "engine-option-codex"
                        })
                        .ghost()
                        .h(px(30.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .items_center()
                                .child(provider_icon(option))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(option.label()),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| this.select_engine(option, window, cx));
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Session permission selector next to the engine picker. FullAccess is
    /// visually flagged (warning colors + alert icon) because it removes the
    /// CLI's own safety gates for this session.
    pub(super) fn permission_picker(&self, view: Entity<Self>) -> AnyElement {
        let permission = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(|session| session.permission)
            .unwrap_or_default();
        let warning = permission == PermissionLevel::FullAccess;
        let trigger = Button::new("permission-picker-trigger")
            .ghost()
            .small()
            .h(px(22.))
            .px_1()
            .cursor_pointer()
            .tooltip(match permission {
                PermissionLevel::ReadOnly => "会话权限：仅可查看",
                PermissionLevel::WorkspaceWrite => "会话权限：工作区内修改",
                PermissionLevel::FullAccess => "会话权限：完全权限（警告：不受沙箱限制）",
            })
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(if warning {
                            gpui_kit::assets::IconName::TriangleAlert
                        } else {
                            gpui_kit::assets::IconName::Shield
                        })
                        .small()
                        .text_color(rgb(if warning {
                            palette().warning_text
                        } else {
                            palette().faint
                        })),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if warning {
                                palette().warning_text
                            } else {
                                palette().muted
                            }))
                            .child(permission.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("permission-picker")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                for option in [
                    PermissionLevel::ReadOnly,
                    PermissionLevel::WorkspaceWrite,
                    PermissionLevel::FullAccess,
                ] {
                    let app = view.clone();
                    let popover = popover.clone();
                    let active = option == permission;
                    let is_danger = option == PermissionLevel::FullAccess;
                    list = list.child(
                        Button::new(match option {
                            PermissionLevel::ReadOnly => "permission-option-readonly",
                            PermissionLevel::WorkspaceWrite => "permission-option-workspace",
                            PermissionLevel::FullAccess => "permission-option-full",
                        })
                        .ghost()
                        .h(px(52.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .child(
                            v_flex()
                                .w_full()
                                .gap(px(2.))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            Icon::new(if is_danger {
                                                gpui_kit::assets::IconName::TriangleAlert
                                            } else {
                                                match option {
                                                    PermissionLevel::ReadOnly => {
                                                        gpui_kit::assets::IconName::Eye
                                                    }
                                                    _ => gpui_kit::assets::IconName::SquarePen,
                                                }
                                            })
                                            .small()
                                            .text_color(rgb(if is_danger {
                                                palette().warning_text
                                            } else {
                                                palette().muted
                                            })),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(rgb(if is_danger {
                                                    palette().warning_text
                                                } else {
                                                    palette().muted
                                                }))
                                                .child(option.label()),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl_6()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(match option {
                                            PermissionLevel::ReadOnly => {
                                                "Codex：只读沙箱；Claude：仅开放读取工具"
                                            }
                                            PermissionLevel::WorkspaceWrite => {
                                                "Codex：工作区沙箱；Claude：自动接受文件编辑"
                                            }
                                            PermissionLevel::FullAccess => {
                                                "警告：不受沙箱限制，命令不经审批直接执行"
                                            }
                                        }),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.set_session_permission(option, window, cx)
                            });
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child("下一条消息生效；正在执行的任务保持原权限"),
                )
            })
            .into_any_element()
    }

    pub(super) fn set_session_permission(
        &mut self,
        permission: PermissionLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        if let Some(id) = self.selected_session
            && let Some(session) = self.session_mut(id)
            && session.permission != permission
        {
            session.permission = permission;
            self.persist();
        }
        // Permission applies to the next turn; while a run is active the
        // current process keeps its existing sandbox.
        if self
            .selected_session
            .is_none_or(|id| !self.runs.contains_key(&id))
        {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn reload_models(&mut self, provider: Provider, cx: &mut Context<Self>) {
        self.catalogs.insert(provider, models::load(provider));
        self.ccs = ccs::load();
        self.store.sync_ccs_providers(&self.ccs);
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
    }

    pub(super) fn set_session_model(
        &mut self,
        id: u64,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        if let Some(session) = self.session_mut(id) {
            session.model = model;
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn composer_model_picker(
        &self,
        view: Entity<Self>,
        session_id: u64,
        provider: Provider,
        selected_model: Option<String>,
        disabled: bool,
    ) -> AnyElement {
        let (label, channel, effort) = self
            .session(session_id)
            .map(|s| {
                let (_, models) = self.channel_models(s.provider, &s.channel);
                let label = selected_model
                    .as_ref()
                    .map(|id| {
                        models
                            .iter()
                            .find(|m| m.id == *id)
                            .map(|m| m.label.clone())
                            .unwrap_or_else(|| id.clone())
                    })
                    .unwrap_or_else(|| "默认模型".into());
                (
                    label,
                    self.channel_label(s.provider, &s.channel),
                    s.reasoning_effort,
                )
            })
            .unwrap_or_else(|| {
                (
                    "默认模型".into(),
                    "CLI 默认".into(),
                    ReasoningEffort::Default,
                )
            });
        let trigger = Button::new("composer-model-trigger")
            .ghost()
            .small()
            .h(px(26.))
            .px_2()
            .rounded(px(7.))
            .disabled(disabled)
            .when(!disabled, |b| b.cursor_pointer())
            .accessibility_label("选择 Provider、模型和思考强度")
            .tooltip(format!("{channel} · 思考强度：{}", effort.label()))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .debug_selector(|| "composer-model-control".into())
                    .gap_2()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(170.))
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(label),
                    )
                    .when(effort != ReasoningEffort::Default, |row| {
                        row.child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(effort.label()),
                        )
                    })
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("composer-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, window, cx| {
                let picker = window.use_keyed_state(
                    ("session-model-picker", session_id),
                    cx,
                    |window, cx| SessionModelPicker::new(view.clone(), session_id, window, cx),
                );
                div().child(picker)
            })
            .into_any_element()
    }

    pub(super) fn channel_label(&self, engine: Provider, channel: &ModelChannel) -> String {
        match channel {
            ModelChannel::Cli => "CLI 默认".into(),
            ModelChannel::Ccs(id) => self.channel_label(
                engine,
                &ModelChannel::Provider(ModelProvider::ccs_id(engine, id)),
            ),
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "Provider 不可用".into()),
        }
    }

    pub(super) fn channel_models(
        &self,
        engine: Provider,
        channel: &ModelChannel,
    ) -> (Option<String>, Vec<models::ModelOption>) {
        match channel {
            ModelChannel::Cli => self
                .catalogs
                .get(&engine)
                .map(|c| (c.error.clone().or_else(|| c.note.clone()), c.models.clone()))
                .unwrap_or_default(),
            ModelChannel::Ccs(id) => self.channel_models(
                engine,
                &ModelChannel::Provider(ModelProvider::ccs_id(engine, id)),
            ),
            ModelChannel::Provider(id) => {
                let models = vec![];
                if let Some(p) = self
                    .store
                    .providers
                    .iter()
                    .find(|p| p.id == *id && p.engine == engine)
                {
                    (None, p.catalog())
                } else {
                    (Some("Provider 已移除，请重新选择".into()), models)
                }
            }
        }
    }

    pub(super) fn session_route(&self, id: u64) -> anyhow::Result<RuntimeRoute> {
        let s = self
            .session(id)
            .ok_or_else(|| anyhow::anyhow!("会话不存在"))?;
        match &s.channel {
            ModelChannel::Cli => Ok(RuntimeRoute::default()),
            ModelChannel::Ccs(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == ModelProvider::ccs_id(s.provider, id))
                .ok_or_else(|| anyhow::anyhow!("Provider 未同步，请刷新模型配置"))?
                .runtime_route(&self.ccs.database_path),
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == s.provider)
                .ok_or_else(|| anyhow::anyhow!("Provider 已移除，请重新选择"))?
                .runtime_route(&self.ccs.database_path),
        }
    }

    /// Model catalog merged with cc-switch synced models, for the Git
    /// generation model picker.
    pub(super) fn git_generation_catalog(&self, provider: Provider) -> ModelCatalog {
        let catalog = self.catalogs.get(&provider);
        let synced = self.ccs.current(provider);
        match (catalog, synced) {
            (Some(catalog), Some(synced)) if !synced.models.is_empty() => {
                catalog.with_synced(&format!("CCS · {}", synced.name), &synced.models)
            }
            (Some(catalog), _) => catalog.clone(),
            (None, Some(synced)) if !synced.models.is_empty() => ModelCatalog::default()
                .with_synced(&format!("CCS · {}", synced.name), &synced.models),
            (None, _) => ModelCatalog::default(),
        }
    }

    /// Picker for the CLI engine used to auto-name sessions.
    pub(super) fn naming_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or("跟随当前会话");
        let trigger = Button::new("naming-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择命名引擎")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 3] = [
                    (None, "跟随当前会话"),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("naming-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_provider = option;
                                    // A model picked for another engine no longer applies.
                                    if option.is_some() {
                                        this.store.ui.naming_model = None;
                                    }
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Picker for what automatic session titling reacts to.
    pub(super) fn naming_mode_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_mode;
        let label = naming_mode_label(current);
        let trigger = Button::new("naming-mode-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择自动重命名方式")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-mode-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(240.)).gap(px(1.));
                for (index, (mode, name, detail)) in [
                    (
                        NamingMode::Latest,
                        "自动根据最新内容重命名",
                        "每轮结束后按最新对话更新标题",
                    ),
                    (
                        NamingMode::FirstMessage,
                        "根据第一条消息内容重命名",
                        "只在第一条消息后生成一次标题",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == mode;
                    list = list.child(
                        Button::new(("naming-mode-option", index))
                            .ghost()
                            .h(px(44.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_mode = mode;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(if active {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(name),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(detail),
                                            ),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Picker for the model used to auto-name sessions.
    pub(super) fn naming_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let follow_session = self.store.ui.naming_provider.is_none();
        let provider = self
            .store
            .ui
            .naming_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.git_generation_catalog(provider);
        let selected_model = self.store.ui.naming_model.clone();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| {
                if follow_session {
                    "跟随当前会话".to_string()
                } else {
                    "CLI 默认".to_string()
                }
            });
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("naming-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择命名模型")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("naming-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let default_label = if follow_session {
                    "跟随当前会话"
                } else {
                    "CLI 默认"
                };
                let options: Vec<(Option<String>, String, String)> =
                    std::iter::once((None, default_label.to_string(), String::new()))
                        .chain(models.iter().map(|model| {
                            (
                                Some(model.id.clone()),
                                model.label.clone(),
                                model.note.clone(),
                            )
                        }))
                        .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    let mark = model_mark(provider, id.as_deref());
                    list = list.child(
                        Button::new(("naming-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(format!("模型 {label}"))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_model = id;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(mark)
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(if selected {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(10.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(format!("{provider_label} · {} 个模型", models.len())),
                );
                list
            })
            .into_any_element()
    }

    /// Settings picker for the CLI used to generate commit messages.
    pub(super) fn git_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.git.generation_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or("跟随当前会话");
        let trigger = Button::new("git-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择生成用 CLI")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 3] = [
                    (None, "跟随当前会话"),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("git-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.generation_provider = option;
                                    // A model picked for another engine no longer applies.
                                    if option.is_some() {
                                        this.store.git.generation_model = None;
                                    }
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Settings picker for the model used to generate commit messages.
    pub(super) fn git_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let provider = self
            .store
            .git
            .generation_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.git_generation_catalog(provider);
        let selected_model = self.store.git.generation_model.clone();
        let follow_session = self.store.git.generation_provider.is_none();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| {
                if follow_session {
                    "跟随当前会话".to_string()
                } else {
                    "CLI 默认".to_string()
                }
            });
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("git-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择生成用模型")
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("git-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let default_label = if follow_session {
                    "跟随当前会话"
                } else {
                    "CLI 默认"
                };
                let options: Vec<(Option<String>, String, String)> =
                    std::iter::once((None, default_label.to_string(), String::new()))
                        .chain(models.iter().map(|model| {
                            (
                                Some(model.id.clone()),
                                model.label.clone(),
                                model.note.clone(),
                            )
                        }))
                        .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    let mark = model_mark(provider, id.as_deref());
                    list = list.child(
                        Button::new(("git-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(format!("模型 {label}"))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.generation_model = id;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(mark)
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(if selected {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(px(10.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(format!("{provider_label} · {} 个模型", models.len())),
                );
                list
            })
            .into_any_element()
    }
}
