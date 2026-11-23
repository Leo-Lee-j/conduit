//! One engine at a time, with task-oriented CLI and model service views.

use super::*;

pub(in crate::app) struct ConnectionsUi {
    pub engine: Provider,
    pub models_expanded: bool,
    pub sources_expanded: bool,
    pub path_help: bool,
    pub syncing: bool,
    pub saving: bool,
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
        let narrow = available_width < 540.;
        let mut actions = h_flex().gap_2().flex_wrap();
        if self.settings_ui.group.is_some() {
            let full = view.clone();
            actions = actions.child(
                Button::new("connections-show-full-page")
                    .debug_selector(|| "settings-show-full-page".into())
                    .small()
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
                                .ghost()
                                .icon(FullIconName::Blocks)
                                .label(conduit::i18n::text_static(
                                    "settings.connections.manage_skills",
                                ))
                                .on_click(move |_, window, cx| {
                                    skills.update(cx, |app, cx| {
                                        app.extensions_ui.engine = engine;
                                        app.select_settings_page(4, window, cx);
                                    })
                                }),
                        )
                        .child(
                            Button::new("settings-view-mcp")
                                .small()
                                .ghost()
                                .icon(FullIconName::Server)
                                .label(conduit::i18n::text_static(
                                    "settings.connections.manage_mcp",
                                ))
                                .on_click(move |_, window, cx| {
                                    mcp.update(cx, |app, cx| {
                                        app.extensions_ui.engine = engine;
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
            available_width,
            self.store.ui.ui_font_size,
            view,
        ))
    }
}
