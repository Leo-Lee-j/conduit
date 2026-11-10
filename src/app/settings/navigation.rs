//! Grouped navigation around the existing setting controls.
use super::*;

pub(in crate::app) struct SettingsUi {
    pub open: bool,
    pub focus: FocusHandle,
    pub focus_before_open: Option<gpui_kit::WeakFocusHandle>,
    pub page: usize,
    pub group: Option<usize>,
    pub search: Entity<InputState>,
    pub naming_expanded: bool,
    pub git_expanded: bool,
    pub prompt_expanded: bool,
}

pub(super) const PAGES: [(&str, &str, &str); 12] = [
    ("工作台", "通用", "布局 内容 会话数量 标签"),
    ("工作台", "外观", "主题 字体 文字 代码"),
    ("工作台", "快捷键", "键盘 shortcut"),
    (
        "AI 连接",
        "CLI 引擎",
        "CLI Claude Codex OpenCode 路径 安装 登录 版本 检查更新 一键更新 升级 进度 upgrade latest update PATH 环境变量 启用 停用 能力",
    ),
    (
        "扩展能力",
        "Skills",
        "skill 技能 插件 导入 编辑 停用 恢复 同名 来源",
    ),
    ("自动化", "自动命名", "会话 标题 模型 触发"),
    ("自动化", "Git 提交", "提交 提示词 模型"),
    ("应用", "数据与更新", "目录 配置文件 下载 重启 诊断 性能"),
    (
        "AI 连接",
        "模型与服务",
        "API Key Base URL Provider CCS cc-switch 凭据 端点 连接 模型 隐藏 恢复 添加 编辑",
    ),
    (
        "扩展能力",
        "MCP 服务",
        "MCP 工具 配置 服务器 添加 编辑 停用 恢复 同名 来源",
    ),
    ("AI 连接", "新会话默认", "默认 模型 服务 引擎 新建 对话"),
    (
        "扩展能力",
        "插件",
        "plugin icons theme install enable disable uninstall 文件图标 主题 安装 启用 停用 卸载",
    ),
];

const PAGE_ORDER: [usize; 12] = [0, 1, 2, 3, 8, 10, 11, 4, 9, 5, 6, 7];

/// The header and navigation share a column, including at larger UI font sizes.
pub(super) fn settings_navigation_width(available_width: f32, font_size: f32) -> f32 {
    let expanded = 224. * (font_size / DEFAULT_UI_FONT_SIZE).max(1.);
    if available_width < expanded + 496. {
        52.
    } else {
        expanded
    }
}

pub(super) const PAGE_SUMMARIES: [&str; 12] = [
    "settings.summary.general",
    "settings.summary.appearance",
    "settings.summary.shortcuts",
    "settings.summary.cli",
    "settings.summary.skills",
    "settings.summary.naming",
    "settings.summary.git",
    "settings.summary.maintenance",
    "settings.summary.models",
    "settings.summary.mcp",
    "settings.summary.defaults",
    "settings.plugins.summary",
];

pub(super) fn matches_search(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

impl ConduitApp {
    pub(super) fn settings_body(
        &self,
        window: &Window,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let font_scale = self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE;
        let navigation_width =
            settings_navigation_width(available_width, self.store.ui.ui_font_size);
        let narrow = navigation_width == 52.;
        let content_width = (available_width - navigation_width).max(0.);
        let query = self.settings_ui.search.read(cx).value();
        let searching = !query.trim().is_empty();
        let mut navigation = v_flex().gap_0p5().w_full();
        let mut group = "";
        for index in PAGE_ORDER {
            let (section, title, _) = PAGES[index];
            let section = conduit::i18n::source(section);
            let title = conduit::i18n::source(title);
            if !narrow && group != section {
                let first_group = group.is_empty();
                group = section;
                navigation = navigation.child(
                    div()
                        .pt(px(if first_group { 8. } else { 16. }))
                        .pb_1()
                        .px_3()
                        .text_size(ui_text_size(12.))
                        .line_height(ui_text_size(16.))
                        .font_medium()
                        .text_color(rgb(palette().muted))
                        .child(section),
                );
            }
            let select = view.clone();
            let icon = match index {
                1 => FullIconName::Palette,
                2 => FullIconName::Keyboard,
                3 => FullIconName::SquareTerminal,
                4 | 9 | 11 => FullIconName::Blocks,
                5 => FullIconName::Pencil,
                6 => FullIconName::GitFork,
                7 => FullIconName::Database,
                8 => FullIconName::Server,
                10 => FullIconName::MessageSquare,
                _ => FullIconName::Settings,
            };
            let selected = !searching && self.settings_ui.page == index;
            navigation = navigation.child(
                Button::new(("settings-navigation", index))
                    .debug_selector(move || format!("settings-nav-{index}"))
                    .accessibility_label(title)
                    .tooltip(title)
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(icon).size(px(16.)).flex_shrink_0())
                            .when(!narrow, |row| {
                                row.child(div().flex_1().min_w_0().truncate().child(title))
                            }),
                    )
                    .ghost()
                    .h(px(32. * font_scale.max(1.)))
                    .w_full()
                    .when(!narrow, |button| button.justify_start())
                    .text_size(ui_text_size(13.))
                    .selected(selected)
                    .when(selected, |button| {
                        button
                            .font_semibold()
                            .text_color(rgb(palette().activity_running))
                    })
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| {
                            app.select_settings_page(index, window, cx);
                        })
                    }),
            );
        }
        let content = if searching {
            self.settings_search_results(&query, view.clone())
        } else if self.settings_ui.page == 11 {
            self.settings_plugins_content(view.clone())
        } else if matches!(self.settings_ui.page, 3 | 8) {
            self.settings_connections_content(content_width, view.clone())
        } else if matches!(self.settings_ui.page, 4 | 9) {
            self.extensions_content(window, content_width, view.clone(), cx)
        } else {
            let page = match self.settings_ui.page {
                1 => self.settings_appearance_page(disabled, view.clone()),
                2 => self.settings_shortcuts_page(disabled, view.clone()),
                5 => self.settings_session_page(disabled, view.clone()),
                6 => self.settings_git_page(disabled, view.clone()),
                7 => self.settings_maintenance_page(disabled, view.clone()),
                10 => self.settings_defaults_page(disabled, view.clone()),
                _ => self.settings_general_page(disabled, view.clone()),
            };
            // Use the library's accessible controls and virtualized page body.
            // Its own navigation is hidden, including its search focus target.
            Settings::new(format!(
                "conduit-settings-page-{}-{:?}",
                self.settings_ui.page, self.settings_ui.group
            ))
            .sidebar_width(px(0.))
            .sidebar_size_range(px(0.)..px(0.))
            .sidebar_style(&StyleRefinement::default().hidden())
            .with_group_variant(GroupBoxVariant::Fill)
            .default_selected_index(gpui_kit::component::setting::SelectIndex {
                page_ix: 0,
                group_ix: None,
            })
            .page(page.into_page(self.settings_ui.group, view))
            .into_any_element()
        };
        h_flex()
            .size_full()
            .min_w_0()
            .items_start()
            .child(
                v_flex()
                    .w(px(navigation_width))
                    .h_full()
                    .flex_shrink_0()
                    .bg(chrome_background(palette().canvas))
                    .border_r_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .id("settings-navigation-scroll")
                            .debug_selector(|| "settings-navigation-scroll".into())
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .px(px(if narrow { 6. } else { 12. }))
                            .pt(px(if narrow { 8. } else { 0. }))
                            .pb_3()
                            .overflow_y_scroll()
                            .child(navigation),
                    )
                    .when(!narrow, |sidebar| {
                        sidebar.child(
                            div()
                                .px_4()
                                .py_3()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(format!("Conduit {}", env!("CARGO_PKG_VERSION"))),
                        )
                    }),
            )
            .child(
                div()
                    .id("settings-content")
                    .debug_selector(|| "settings-content".into())
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(content),
            )
            .into_any_element()
    }

    pub(super) fn select_settings_page(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= PAGES.len() {
            return;
        }
        if self.settings_ui.page != index {
            self.extensions_ui.cancel_mcp_detection();
        }
        self.settings_ui.page = index;
        self.settings_ui.group = None;
        self.settings_ui
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if matches!(index, 4 | 9) {
            self.extensions_ui.mcp = index == 9;
            self.ensure_extensions(window, cx, false);
        } else {
            self.extensions_ui.probe = None;
            self.extensions_ui.connection_probe = None;
        }
        cx.notify();
    }

    pub(super) fn generation_summary(&self, naming: bool) -> String {
        let (provider, channel, model) = if naming {
            (
                self.store.ui.naming_provider,
                &self.store.ui.naming_channel,
                &self.store.ui.naming_model,
            )
        } else {
            (
                self.store.git.generation_provider,
                &self.store.git.generation_channel,
                &self.store.git.generation_model,
            )
        };
        if provider.is_none() && channel.is_none() && model.is_none() {
            return conduit::i18n::text_static("generation.follow_current_session").into();
        }
        let route = match channel {
            Some(ModelChannel::Provider(id)) => self
                .store
                .providers
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.as_str())
                .unwrap_or(conduit::i18n::text_static(
                    "settings.navigation.service_unavailable",
                )),
            Some(ModelChannel::Ccs(_)) => "CCS",
            Some(ModelChannel::Cli) => conduit::i18n::text_static("generation.cli_default"),
            None => {
                if provider.is_some() {
                    conduit::i18n::text_static("generation.cli_default")
                } else {
                    conduit::i18n::text_static("settings.navigation.follow_session_service")
                }
            }
        };
        format!(
            "{} · {} · {}",
            provider
                .map(Provider::label)
                .unwrap_or(conduit::i18n::text_static(
                    "settings.navigation.follow_session_engine"
                )),
            route,
            model
                .as_deref()
                .unwrap_or(conduit::i18n::text_static("model_picker.default_model"))
        )
    }

    pub(super) fn generation_disclosure(&self, naming: bool, view: Entity<Self>) -> SettingItem {
        let expanded = if naming {
            self.settings_ui.naming_expanded
        } else {
            self.settings_ui.git_expanded
        };
        let summary = self.generation_summary(naming);
        SettingItem::render(move |_, _, _| {
            let view = view.clone();
            h_flex()
                .w_full()
                .gap_2()
                .justify_between()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(ui_text_size(12.))
                        .child(summary.clone()),
                )
                .child(
                    Button::new(if naming {
                        "naming-config-expand"
                    } else {
                        "git-config-expand"
                    })
                    .small()
                    .ghost()
                    .label(if expanded {
                        conduit::i18n::text_static("settings.navigation.hide_configuration")
                    } else {
                        conduit::i18n::text_static("settings.navigation.configure_model")
                    })
                    .on_click(move |_, _, cx| {
                        view.update(cx, |app, cx| {
                            if naming {
                                app.settings_ui.naming_expanded = !expanded;
                            } else {
                                app.settings_ui.git_expanded = !expanded;
                            }
                            cx.notify();
                        });
                    }),
                )
        })
        .keywords([
            conduit::i18n::text_static("model_picker.model_98fd0c"),
            conduit::i18n::text_static("model_picker.engine"),
            "Provider",
        ])
    }
}
