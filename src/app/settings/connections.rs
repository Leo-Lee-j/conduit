//! One engine at a time, with task-oriented CLI and model service views.

use super::*;

pub(in crate::app) struct ConnectionsUi {
    pub engine: Provider,
    pub models_expanded: bool,
    pub sources_expanded: bool,
    pub path_help: bool,
    pub syncing: bool,
    pub saving: bool,
    pub(super) engine_detail_open: bool,
    pub(super) engine_scroll: ScrollHandle,
    pub(super) engine_detail_scroll: ScrollHandle,
    pub model_search: Entity<InputState>,
    pub model_limit: usize,
    pub(super) service_selection: Option<(Provider, ModelChannel)>,
    pub(super) service_detail_open: bool,
    pub(super) service_scroll: ScrollHandle,
    pub(super) service_detail_scroll: ScrollHandle,
    pub(super) service_tab: super::services::DetailTab,
    pub(super) cli_updates: HashMap<Provider, super::cli_settings::CliUpdateAction>,
}

impl ConnectionsUi {
    pub(in crate::app) fn new(engine: Provider, model_search: Entity<InputState>) -> Self {
        Self {
            engine,
            models_expanded: false,
            sources_expanded: false,
            path_help: false,
            syncing: false,
            saving: false,
            engine_detail_open: false,
            engine_scroll: ScrollHandle::new(),
            engine_detail_scroll: ScrollHandle::new(),
            model_search,
            model_limit: 50,
            service_selection: None,
            service_detail_open: false,
            service_scroll: ScrollHandle::new(),
            service_detail_scroll: ScrollHandle::new(),
            service_tab: super::services::DetailTab::Models,
            cli_updates: HashMap::new(),
        }
    }
}

impl ConduitApp {
    pub(super) fn settings_connections_content(
        &self,
        available_width: f32,
        view: Entity<Self>,
    ) -> SettingsContent {
        let engine = self.connection_settings.engine;
        let services = self.settings_ui.page == 8;
        let disabled = !self.writable || self.closing || self.connection_settings.saving;
        let scale = self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE;
        let narrow = available_width < 540. * scale;
        let mut actions = h_flex().gap_2().flex_wrap();
        if self.settings_ui.group.is_some() {
            let full = view.clone();
            actions = actions.child(
                Button::new("connections-show-full-page")
                    .debug_selector(|| "settings-show-full-page".into())
                    .small()
                    .h(ui_text_size(32.))
                    .text_size(ui_text_size(14.))
                    .ghost()
                    .label(conduit::i18n::text_static(
                        "settings.connections.view_full_page",
                    ))
                    .on_click(move |_, window, cx| {
                        full.update(cx, |app, cx| {
                            app.select_settings_page(app.settings_ui.page, window, cx)
                        })
                    }),
            );
        }
        if services {
            let refresh = view.clone();
            actions = actions.child(
                Button::new("settings-models-refresh")
                    .small()
                    .h(ui_text_size(32.))
                    .text_size(ui_text_size(14.))
                    .outline()
                    .icon(FullIconName::RefreshCw)
                    .accessibility_label(conduit::i18n::text_static(
                        "settings.connections.reload_configuration",
                    ))
                    .tooltip(if self.connection_settings.syncing {
                        conduit::i18n::text_static("settings.connections.reading")
                    } else {
                        conduit::i18n::text_static("settings.connections.reload_configuration")
                    })
                    .when(!narrow, |button| {
                        button.label(if self.connection_settings.syncing {
                            conduit::i18n::text_static("settings.connections.reading")
                        } else {
                            conduit::i18n::text_static("settings.connections.reload_configuration")
                        })
                    })
                    .disabled(
                        self.closing
                            || self.connection_settings.syncing
                            || self.connection_settings.saving,
                    )
                    .on_click(move |_, window, cx| {
                        refresh.update(cx, |app, cx| app.reload_all_models_async(true, window, cx))
                    }),
            );
            if engine.descriptor().capabilities.model_services {
                let add = view.clone();
                actions = actions.child(
                    Button::new("add-custom-provider")
                        .debug_selector(|| "settings-add-service".into())
                        .small()
                        .h(ui_text_size(32.))
                        .text_size(ui_text_size(14.))
                        .primary()
                        .icon(FullIconName::Plus)
                        .accessibility_label(conduit::i18n::text_static(
                            "settings.connections.add_service",
                        ))
                        .tooltip(conduit::i18n::text_static(
                            "settings.connections.add_service",
                        ))
                        .when(!narrow, |button| {
                            button.label(conduit::i18n::text_static(
                                "settings.connections.add_service",
                            ))
                        })
                        .disabled(disabled)
                        .on_click(move |_, window, cx| {
                            add.update(cx, |app, cx| app.open_provider_editor(None, window, cx))
                        }),
                );
            }
        }
        if services && self.settings_ui.group.is_none() {
            return self.model_services_page(available_width, actions.into_any_element(), view);
        }
        if !services && self.settings_ui.group.is_none() {
            return self.settings_engines_content(available_width, disabled, view);
        }
        let mut groups = vec![];
        if services {
            if engine.descriptor().capabilities.model_services {
                groups.push(self.providers_group(engine, disabled, view.clone()));
            }
            groups.push(self.models_provider_group(engine, view.clone()));
            if engine.descriptor().capabilities.model_services {
                groups.push(self.models_source_group(engine, view.clone()));
            }
        } else {
            groups.push(self.settings_provider_group(engine, disabled, view.clone()));
            let links = view.clone();
            groups.push(SettingGroup::new().variant(GroupBoxVariant::Normal).item(
                SettingItem::render(move |_, _, _| {
                    let skills = links.clone();
                    let mcp = links.clone();
                    h_flex()
                        .w_full()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("settings-view-extensions")
                                .small()
                                .h(ui_text_size(32.))
                                .text_size(ui_text_size(14.))
                                .ghost()
                                .icon(FullIconName::Blocks)
                                .label(conduit::i18n::text_static(
                                    "settings.connections.manage_skills",
                                ))
                                .on_click(move |_, window, cx| {
                                    skills.update(cx, |app, cx| {
                                        app.select_settings_engine(engine, window, cx);
                                        app.select_settings_page(4, window, cx);
                                    })
                                }),
                        )
                        .child(
                            Button::new("settings-view-mcp")
                                .small()
                                .h(ui_text_size(32.))
                                .text_size(ui_text_size(14.))
                                .ghost()
                                .icon(FullIconName::Server)
                                .label(conduit::i18n::text_static(
                                    "settings.connections.manage_mcp",
                                ))
                                .on_click(move |_, window, cx| {
                                    mcp.update(cx, |app, cx| {
                                        app.select_settings_engine(engine, window, cx);
                                        app.select_settings_page(9, window, cx);
                                    })
                                }),
                        )
                }),
            ));
        }
        let page = SettingPage {
            page: gpui_kit::component::setting::SettingPage::new("")
                .resettable(false)
                .header_style(&StyleRefinement::default().hidden()),
            groups,
        };
        SettingsContent::new(settings_controls(
            format!(
                "conduit-connections-{}-{:?}-{:?}",
                self.settings_ui.page, engine, self.settings_ui.group
            ),
            page.into_page(self.settings_ui.group, view.clone())
                .header_style(&StyleRefinement::default().hidden()),
        ))
        .with_toolbar(management_toolbar(
            engine,
            false,
            None,
            (services || self.settings_ui.group.is_some()).then(|| actions.into_any_element()),
            view,
        ))
    }

    fn settings_engines_content(
        &self,
        available_width: f32,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingsContent {
        let engine = self.connection_settings.engine;
        let scale = self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE;
        let split = available_width - settings_content_inset() * 2. >= 720. * scale;
        let detail_only = !split && self.connection_settings.engine_detail_open;
        let mut list = v_flex()
            .id("settings-engine-list")
            .debug_selector(|| "settings-engine-list".into())
            .w_full()
            .min_w_0()
            .refine_style(&settings_group_style().p_0().border_0());
        for (index, candidate) in Provider::ALL.into_iter().enumerate() {
            let select = view.clone();
            let enabled = self.store.cli.enabled(candidate);
            list = list.child(
                Button::new(("management-engine", index))
                    .debug_selector(move || format!("settings-engine-{index}"))
                    .accessibility_label(candidate.label())
                    .tooltip(candidate.label())
                    .ghost()
                    .selected(candidate == engine)
                    .w_full()
                    .min_w_0()
                    .h_auto()
                    .px_2()
                    .py_2()
                    .rounded(px(4.))
                    .border_b_1()
                    .border_color(settings_group_border())
                    .when(candidate == engine, |row| {
                        row.bg(gpui_kit::rgba((palette().activity_running << 8) | 0x0c))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .child(provider_icon(candidate))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_medium()
                                            .text_size(ui_text_size(14.))
                                            .child(candidate.label()),
                                    )
                                    .child(
                                        div()
                                            .text_size(ui_text_size(12.))
                                            .text_color(rgb(palette().muted))
                                            .child(conduit::i18n::text_static(if enabled {
                                                "settings.plugins.enabled"
                                            } else {
                                                "settings.plugins.disabled"
                                            })),
                                    ),
                            ),
                    )
                    .on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| {
                            if app.connection_settings.engine != candidate {
                                app.connection_settings
                                    .engine_detail_scroll
                                    .set_offset(point(px(0.), px(0.)));
                            }
                            app.select_settings_engine(candidate, window, cx);
                            app.connection_settings.engine_detail_open = true;
                            cx.notify();
                        })
                    }),
            );
        }
        let back = view.clone();
        let skills = view.clone();
        let mcp = view.clone();
        let body = settings_scroll_area("settings-engines-scroll")
            .text_size(ui_text_size(14.))
            .track_scroll(if detail_only {
                &self.connection_settings.engine_detail_scroll
            } else {
                &self.connection_settings.engine_scroll
            })
            .child(
                h_flex()
                    .debug_selector(|| "engine-workbench".into())
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_4()
                    .when(!detail_only, |row| {
                        row.child(
                            div()
                                .min_w_0()
                                .flex_shrink_0()
                                .when(split, |column| {
                                    column
                                        .w(ui_text_size(248.))
                                        .pr_3()
                                        .border_r_1()
                                        .border_color(settings_group_border())
                                })
                                .when(!split, |column| column.w_full().max_w(ui_text_size(440.)))
                                .child(list),
                        )
                    })
                    .when(split || detail_only, |row| {
                        row.child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .max_w(ui_text_size(680.))
                                .gap_3()
                                .when(detail_only, |detail| {
                                    detail.child(
                                        h_flex().child(
                                            Button::new("engine-detail-back")
                                                .debug_selector(|| "engine-detail-back".into())
                                                .small()
                                                .h(ui_text_size(32.))
                                                .ghost()
                                                .icon(FullIconName::ArrowLeft)
                                                .label(conduit::i18n::text_static(
                                                    "settings.cli.back_to_list",
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    back.update(cx, |app, cx| {
                                                        app.connection_settings
                                                            .engine_detail_open = false;
                                                        cx.notify();
                                                    })
                                                }),
                                        ),
                                    )
                                })
                                .child(self.settings_provider_detail(
                                    engine,
                                    disabled,
                                    view.clone(),
                                ))
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_wrap()
                                        .pt_2()
                                        .border_t_1()
                                        .border_color(settings_group_border())
                                        .child(
                                            Button::new("settings-view-extensions")
                                                .small()
                                                .h(ui_text_size(32.))
                                                .ghost()
                                                .icon(FullIconName::Blocks)
                                                .label(conduit::i18n::text_static(
                                                    "settings.connections.manage_skills",
                                                ))
                                                .on_click(move |_, window, cx| {
                                                    skills.update(cx, |app, cx| {
                                                        app.select_settings_engine(
                                                            engine, window, cx,
                                                        );
                                                        app.select_settings_page(4, window, cx);
                                                    })
                                                }),
                                        )
                                        .child(
                                            Button::new("settings-view-mcp")
                                                .small()
                                                .h(ui_text_size(32.))
                                                .ghost()
                                                .icon(FullIconName::Server)
                                                .label(conduit::i18n::text_static(
                                                    "settings.connections.manage_mcp",
                                                ))
                                                .on_click(move |_, window, cx| {
                                                    mcp.update(cx, |app, cx| {
                                                        app.select_settings_engine(
                                                            engine, window, cx,
                                                        );
                                                        app.select_settings_page(9, window, cx);
                                                    })
                                                }),
                                        ),
                                ),
                        )
                    }),
            );
        SettingsContent::new(body)
    }
}
