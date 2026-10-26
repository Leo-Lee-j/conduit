//! Provider, permission, and model selection controls.

use super::*;

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
                        .h(px(34.))
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
                                                "CLI 沙箱只读，不允许写入与命令执行"
                                            }
                                            PermissionLevel::WorkspaceWrite => {
                                                "允许修改工作区内文件，其余仍需审批"
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
                list
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
        cx.notify();
    }

    pub(super) fn reload_all_models(&mut self, cx: &mut Context<Self>) {
        for provider in [Provider::Claude, Provider::Codex] {
            self.catalogs.insert(provider, models::load(provider));
        }
        self.ccs = ccs::load();
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
        let catalog = self.catalogs.get(&provider);
        let synced = self.ccs.current(provider);
        let merged = match (catalog, synced) {
            (Some(catalog), Some(synced)) if !synced.models.is_empty() => {
                catalog.with_synced(&format!("CCS · {}", synced.name), &synced.models)
            }
            (Some(catalog), _) => catalog.clone(),
            (None, Some(synced)) if !synced.models.is_empty() => ModelCatalog::default()
                .with_synced(&format!("CCS · {}", synced.name), &synced.models),
            (None, _) => ModelCatalog::default(),
        };
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| "CLI 默认".into());
        let models = merged.models;
        let mut notes = vec![];
        if let Some(note) =
            catalog.and_then(|catalog| catalog.error.clone().or_else(|| catalog.note.clone()))
        {
            notes.push(format!("配置：{note}"));
        }
        if let Some(synced) = synced {
            notes.push(format!(
                "CCS：{} · {} 个模型",
                synced.name,
                synced.models.len()
            ));
        } else if let Some(error) = &self.ccs.error {
            notes.push(format!("CCS：{error}"));
        } else if let Some(note) = &self.ccs.note {
            notes.push(format!("CCS：{note}"));
        }
        let sync_note = (!notes.is_empty()).then(|| notes.join("；"));
        let provider_label = provider.label();
        let trigger = Button::new("composer-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label("选择模型")
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(160.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(trigger_label),
                    )
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
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let model_count = models.len();
                let mut list = v_flex().w(px(340.)).gap_1();
                list = list.child(
                    h_flex()
                        .px_2()
                        .pt_1()
                        .pb_2()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_semibold()
                                .text_color(rgb(palette().text))
                                .child("会话模型"),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(format!("{provider_label} · {model_count} 个")),
                        ),
                );
                let option = |index: usize,
                              id: Option<String>,
                              label: String,
                              note: String,
                              selected: bool|
                 -> AnyElement {
                    let view = view.clone();
                    let popover = popover.clone();
                    let mark = model_mark(provider, id.as_deref());
                    Button::new(("model-option", index))
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
                                this.set_session_model(session_id, id, cx);
                            });
                            if let Some(state) = popover.clone().upgrade() {
                                state.update(cx, |state, cx| state.dismiss(window, cx));
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
                                .child(Icon::new(IconName::Check).small().text_color(rgb(
                                    if selected {
                                        palette().text
                                    } else {
                                        palette().surface_hover
                                    },
                                ))),
                        )
                        .into_any_element()
                };
                list = list.child(option(
                    0,
                    None,
                    "CLI 默认".into(),
                    format!("跟随 {provider_label} 当前配置"),
                    selected_model.is_none(),
                ));
                for (index, model) in models.iter().enumerate() {
                    let mut note = vec![];
                    if model.label != model.id {
                        note.push(model.id.clone());
                    }
                    if !model.note.is_empty() {
                        note.push(model.note.clone());
                    }
                    list = list.child(option(
                        index + 1,
                        Some(model.id.clone()),
                        model.label.clone(),
                        note.join(" · "),
                        selected_model.as_deref() == Some(model.id.as_str()),
                    ));
                }
                if models.is_empty() {
                    list = list.child(
                        div()
                            .px_2()
                            .py_2()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child("未发现模型定义；可在左下角「设置」中重新读取。"),
                    );
                }
                if let Some(note) = &sync_note {
                    list = list.child(
                        div()
                            .px_2()
                            .pt_1()
                            .text_size(px(10.))
                            .line_height(relative(1.4))
                            .text_color(rgb(palette().faint))
                            .child(note.clone()),
                    );
                }
                let refresh_view = view.clone();
                let refresh_popover = popover.clone();
                list.child(
                    h_flex()
                        .px_2()
                        .pt_2()
                        .gap_2()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child("仅影响本会话，下一条消息生效"),
                        )
                        .child(
                            h_flex()
                                .id("model-refresh")
                                .cursor_pointer()
                                .rounded(px(4.))
                                .px_1()
                                .text_size(px(10.))
                                .text_color(rgb(palette().muted))
                                .hover(|style| style.bg(rgb(palette().surface_hover)))
                                .on_click(move |_, window, cx| {
                                    refresh_view.update(cx, |this, cx| {
                                        this.reload_models(provider, cx);
                                    });
                                    if let Some(state) = refresh_popover.clone().upgrade() {
                                        state.update(cx, |state, cx| state.dismiss(window, cx));
                                    }
                                })
                                .child("重新读取配置"),
                        ),
                )
            })
            .into_any_element()
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
