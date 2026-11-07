//! One engine at a time, with task-oriented CLI and model service views.

use super::*;

pub(in crate::app) struct ConnectionsUi {
    pub engine: Provider,
    pub models_expanded: bool,
    pub sources_expanded: bool,
    pub path_help: bool,
    pub syncing: bool,
    pub model_search: Entity<InputState>,
    pub model_limit: usize,
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
            model_search,
            model_limit: 50,
            cli_updates: HashMap::new(),
        }
    }
}

impl ConduitApp {
    pub(in crate::app) fn settings_connections_content(
        &self,
        window: &Window,
        available_width: f32,
        view: Entity<Self>,
    ) -> AnyElement {
        let engine = self.connection_settings.engine;
        let services = self.settings_ui.page == 8;
        let disabled = !self.writable || self.closing;
        let compact = f32::from(window.viewport_size().height) < 650.;
        let narrow = available_width < 540.;
        let mut actions = h_flex().gap_2().flex_wrap();
        if self.settings_ui.group.is_some() {
            let full = view.clone();
            actions = actions.child(
                Button::new("connections-show-full-page")
                    .debug_selector(|| "settings-show-full-page".into())
                    .small()
                    .ghost()
                    .label("查看完整页面")
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
                    .accessibility_label("重新读取配置")
                    .tooltip(if self.connection_settings.syncing {
                        "读取中…"
                    } else {
                        "重新读取配置"
                    })
                    .when(!narrow, |button| {
                        button.label(if self.connection_settings.syncing {
                            "读取中…"
                        } else {
                            "重新读取配置"
                        })
                    })
                    .disabled(self.closing || self.connection_settings.syncing)
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
                        .accessibility_label("添加服务")
                        .tooltip("添加服务")
                        .when(!narrow, |button| button.label("添加服务"))
                        .disabled(disabled)
                        .on_click(move |_, window, cx| {
                            add.update(cx, |app, cx| app.open_provider_editor(None, window, cx))
                        }),
                );
            }
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
                                .icon(FullIconName::Sparkles)
                                .label("管理 Skills")
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
                                .label("管理 MCP")
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
        let title = if services {
            "Model · 模型与服务"
        } else {
            "CLI · 引擎管理"
        };
        let description = if services {
            "管理服务端点与模型，设置新会话使用的服务。"
        } else {
            "连接本机 AI 编程工具，管理启动路径、版本与启用状态。"
        };
        let page = SettingPage {
            page: gpui_kit::component::setting::SettingPage::new("")
                .resettable(false)
                .header_style(&StyleRefinement::default().hidden()),
            groups,
        };
        v_flex()
            .size_full()
            .min_w_0()
            .child(management_header(
                title,
                description,
                if services {
                    FullIconName::Boxes
                } else {
                    FullIconName::SquareTerminal
                },
                actions.into_any_element(),
                compact || narrow,
                narrow,
            ))
            .child(
                div()
                    .px(px(if narrow { 12. } else { 24. }))
                    .pb_3()
                    .child(engine_tabs(engine, false, narrow, view.clone())),
            )
            .child(
                div().flex_1().min_h_0().w_full().child(
                    Settings::new(format!(
                        "conduit-connections-{}-{:?}-{:?}",
                        self.settings_ui.page, engine, self.settings_ui.group
                    ))
                    .sidebar_width(px(0.))
                    .sidebar_size_range(px(0.)..px(0.))
                    .sidebar_style(&StyleRefinement::default().hidden())
                    .with_group_variant(GroupBoxVariant::Normal)
                    .page(page.into_page(self.settings_ui.group, view)),
                ),
            )
            .into_any_element()
    }
}
