//! One engine at a time, with task-oriented CLI and model service views.

use super::*;

pub(in crate::app) struct ConnectionsUi {
    pub engine: Provider,
    pub models_expanded: bool,
    pub sources_expanded: bool,
    pub path_help: bool,
    pub syncing: bool,
}

impl ConnectionsUi {
    pub(in crate::app) fn new(engine: Provider) -> Self {
        Self {
            engine,
            models_expanded: false,
            sources_expanded: false,
            path_help: false,
            syncing: false,
        }
    }
}

impl ConduitApp {
    pub(in crate::app) fn settings_connections_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let engine = self.connection_settings.engine;
        let services = self.settings_ui.page == 8;
        let selector_view = view.clone();
        let selector = SettingItem::render(move |_, _, _| {
            let mut engines = h_flex().w_full().gap_2();
            for (index, candidate) in Provider::ALL.into_iter().enumerate() {
                let select_view = selector_view.clone();
                engines = engines.child(
                    Button::new(("settings-engine", index))
                        .debug_selector(move || format!("settings-engine-{index}"))
                        .flex_1()
                        .min_w_0()
                        .h(px(32.))
                        .outline()
                        .label(candidate.label())
                        .selected(candidate == engine)
                        .when(candidate == engine, |button| {
                            button
                                .border_color(rgb(palette().activity_running))
                                .text_color(rgb(palette().activity_running))
                        })
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            select_view.update(cx, |app, cx| {
                                app.connection_settings.engine = candidate;
                                app.connection_settings.models_expanded = false;
                                app.connection_settings.sources_expanded = false;
                                cx.notify();
                            });
                        }),
                );
            }
            engines
        })
        .keywords([
            "CLI", "模型", "服务", "Claude", "Codex", "OpenCode", "Provider", "CCS", "路径", "版本",
        ]);
        let mut groups = vec![
            SettingGroup::new()
                .variant(GroupBoxVariant::Normal)
                .item(selector),
        ];
        if services {
            if engine != Provider::OpenCode {
                groups.push(self.providers_group(engine, disabled, view.clone()));
            }
            groups.push(self.models_provider_group(engine, view.clone()));
            if engine != Provider::OpenCode {
                groups.push(self.models_source_group(engine, view));
            }
        } else {
            groups.push(self.settings_provider_group(engine, disabled, view.clone()));
            let extensions_view = view.clone();
            groups.push(
                SettingGroup::new().variant(GroupBoxVariant::Normal).item(
                    SettingItem::render(move |_, _, _| {
                        let view = extensions_view.clone();
                        Button::new("settings-view-extensions")
                            .small()
                            .ghost()
                            .label("管理 Skills")
                            .on_click(move |_, window, cx| {
                                view.update(cx, |app, cx| {
                                    app.extensions_ui.engine = engine;
                                    app.select_settings_page(4, window, cx);
                                });
                            })
                    })
                    .keywords(["Skills", "MCP", "扩展"]),
                ),
            );
            let expanded = self.connection_settings.path_help;
            let help = SettingItem::render(move |_, _, _| {
                let help_view = view.clone();
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex().w_full().justify_start().child(Button::new("settings-path-help")
                            .ghost()
                            .small()
                            .label(if expanded { "收起路径查找说明" } else { "如何查找 CLI？" })
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                help_view.update(cx, |app, cx| {
                                    app.connection_settings.path_help = !expanded;
                                    cx.notify();
                                });
                            })),
                    )
                    .when(expanded, |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .line_height(relative(1.6))
                                .text_color(rgb(palette().muted))
                                .child("查找顺序：环境变量 → 应用指定路径 → PATH → 常见安装位置。环境变量 CONDUIT_CLAUDE_PATH / CONDUIT_CODEX_PATH / CONDUIT_OPENCODE_PATH 优先于这里的设置。支持 .exe 和 npm 全局 .cmd 启动器。"),
                        )
                    })
            })
            .keywords(["路径", "环境变量", "PATH"]);
            groups.push(
                SettingGroup::new()
                    .variant(GroupBoxVariant::Normal)
                    .item(help),
            );
        }
        settings_page(
            if services {
                "模型与服务"
            } else {
                "AI 引擎"
            },
            FullIconName::SquareTerminal,
        )
        .header_style(&page_header_style().py(px(12.)))
        .groups(groups)
    }
}
