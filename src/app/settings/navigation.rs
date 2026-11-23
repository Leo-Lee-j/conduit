//! Stable settings routes with grouped, text-first navigation.
use super::*;

pub(in crate::app) struct SettingsUi {
    pub open: bool,
    pub focus: FocusHandle,
    pub focus_before_open: Option<gpui_kit::WeakFocusHandle>,
    /// Legacy numeric identities are retained for direct entry points and extensions.
    pub page: usize,
    pub group: Option<usize>,
    /// Track reflow without changing persistent settings or recreating editors.
    pub layout_compact: std::cell::Cell<Option<bool>>,
    /// A dialog may outlive the navigation control that owned its return focus.
    pub restore_navigation_focus: std::cell::Cell<bool>,
    pub search: Entity<InputState>,
    pub naming_expanded: bool,
    pub git_expanded: bool,
    pub prompt_expanded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(in crate::app) enum SettingsRoute {
    General = 0,
    Appearance = 1,
    Shortcuts = 2,
    Engines = 3,
    Skills = 4,
    AutomaticNaming = 5,
    Git = 6,
    Maintenance = 7,
    Services = 8,
    Mcp = 9,
    Sessions = 10,
    Plugins = 11,
}

impl SettingsRoute {
    const ALL: [Self; 12] = [
        Self::General,
        Self::Appearance,
        Self::Shortcuts,
        Self::Engines,
        Self::Skills,
        Self::AutomaticNaming,
        Self::Git,
        Self::Maintenance,
        Self::Services,
        Self::Mcp,
        Self::Sessions,
        Self::Plugins,
    ];

    pub(super) fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub(in crate::app) const fn index(self) -> usize {
        self as usize
    }

    /// Automatic naming remains a valid legacy route under the session category.
    pub(super) fn navigation_index(self) -> usize {
        match self {
            Self::AutomaticNaming => Self::Sessions.index(),
            _ => self.index(),
        }
    }

    fn extensions(self) -> bool {
        matches!(self, Self::Skills | Self::Mcp)
    }
}

pub(super) const PAGES: [(&str, &str, &str); 12] = [
    ("工作台", "通用", "布局 内容 会话数量 标签"),
    (
        "工作台",
        "外观与字体",
        "外观 主题 字体 文字 代码 appearance typography",
    ),
    ("工作台", "快捷键", "键盘 shortcut"),
    (
        "AI 与工作流",
        "AI 引擎",
        "CLI Claude Codex OpenCode Qoder 路径 安装 登录 版本 检查更新 一键更新 升级 进度 upgrade latest update PATH 环境变量 启用 停用 能力",
    ),
    (
        "扩展能力",
        "Skills",
        "skill 技能 插件 导入 编辑 停用 恢复 同名 来源",
    ),
    ("AI 与工作流", "自动命名", "会话 标题 模型 触发 自动化"),
    (
        "AI 与工作流",
        "Git 工作流",
        "Git 提交 提交说明 提示词 模型 自动化",
    ),
    (
        "应用维护",
        "数据与更新",
        "目录 配置文件 下载 重启 诊断 性能",
    ),
    (
        "AI 与工作流",
        "模型与服务",
        "API Key Base URL Provider CCS cc-switch 凭据 端点 连接 模型 隐藏 恢复 添加 编辑",
    ),
    (
        "扩展能力",
        "MCP 服务",
        "MCP 工具 配置 服务器 添加 编辑 停用 恢复 同名 来源",
    ),
    (
        "AI 与工作流",
        "会话设置",
        "新会话默认 默认 模型 服务 引擎 新建 对话 权限 聊天 显示 折叠 自动命名 标题 session permission naming",
    ),
    (
        "扩展能力",
        "插件",
        "plugin icons theme install enable disable uninstall 文件图标 主题 安装 启用 停用 卸载",
    ),
];

const PAGE_ORDER: [usize; 11] = [0, 1, 2, 3, 8, 10, 6, 4, 9, 11, 7];

/// Use the actual settings pane and UI font, not the outer window width.
/// A compact pane has a full text category selector instead of an icon-only rail.
pub(super) fn settings_navigation_width(available_width: f32, font_size: f32) -> f32 {
    let scale = (font_size / DEFAULT_UI_FONT_SIZE).max(1.);
    let expanded = 224. * scale;
    if available_width < expanded + 480. * scale {
        0.
    } else {
        expanded
    }
}

pub(super) fn matches_search(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| text.contains(word))
}

impl ConduitApp {
    pub(super) fn settings_category_selector(&self, view: Entity<Self>) -> AnyElement {
        let route =
            SettingsRoute::from_index(self.settings_ui.page).unwrap_or(SettingsRoute::General);
        let (section, title, _) = PAGES[route.navigation_index()];
        let label = format!(
            "{} / {}",
            conduit::i18n::source(section),
            conduit::i18n::source(title)
        );
        Button::new("settings-category-selector")
            .debug_selector(|| "settings-category-selector".into())
            .accessibility_label(conduit::i18n::text_static(
                "settings.workspace.choose_category",
            ))
            .outline()
            .w_full()
            .min_w_0()
            .h(px(36.
                * (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE)
                    .max(1.)))
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(div().flex_1().min_w_0().truncate().child(label))
                    .child(Icon::new(FullIconName::ChevronDown).small().flex_shrink_0()),
            )
            .dropdown_menu(move |mut menu, _, _| {
                for index in PAGE_ORDER {
                    let select = view.clone();
                    let (section, title, _) = PAGES[index];
                    let label = format!(
                        "{} / {}",
                        conduit::i18n::source(section),
                        conduit::i18n::source(title)
                    );
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| app.select_settings_page(index, window, cx));
                    }));
                }
                menu
            })
            .into_any_element()
    }

    fn settings_navigation(
        &self,
        route: SettingsRoute,
        searching: bool,
        view: Entity<Self>,
    ) -> impl IntoElement {
        let font_scale = (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE).max(1.);
        let mut navigation = v_flex().gap_1().w_full();
        let mut group = "";
        for index in PAGE_ORDER {
            let (section, title, _) = PAGES[index];
            let section = conduit::i18n::source(section);
            let title = conduit::i18n::source(title);
            if group != section {
                let first_group = group.is_empty();
                group = section;
                navigation = navigation.child(
                    div()
                        .pt(px(if first_group { 12. } else { 20. }))
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
                6 => FullIconName::GitFork,
                7 => FullIconName::Database,
                8 => FullIconName::Server,
                10 => FullIconName::MessageSquare,
                _ => FullIconName::Settings,
            };
            let selected = !searching && route.navigation_index() == index;
            navigation = navigation.child(
                Button::new(("settings-navigation", index))
                    .debug_selector(move || format!("settings-nav-{index}"))
                    .accessibility_label(title)
                    .tooltip(title)
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .child(Icon::new(icon).size(px(16. * font_scale)).flex_shrink_0())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .line_height(ui_text_size(18.))
                                    .child(title),
                            ),
                    )
                    .ghost()
                    .h_auto()
                    .min_h(px(36. * font_scale))
                    .py(px(7. * font_scale))
                    .w_full()
                    .justify_start()
                    .text_size(ui_text_size(13.))
                    .selected(selected)
                    .when(selected, |button| {
                        button.font_medium().text_color(rgb(palette().text))
                    })
                    .on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| app.select_settings_page(index, window, cx));
                    }),
            );
        }
        navigation
    }

    pub(super) fn settings_body(
        &self,
        window: &Window,
        navigation_width: f32,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let disabled = !self.writable || self.closing;
        let compact = navigation_width == 0.;
        let content_width = (available_width - navigation_width).max(0.);
        let route =
            SettingsRoute::from_index(self.settings_ui.page).unwrap_or(SettingsRoute::General);
        let query = self.settings_ui.search.read(cx).value();
        let searching = !query.trim().is_empty();
        let content = if searching {
            SettingsContent::new(self.settings_search_results(&query, view.clone()))
        } else {
            match route {
                SettingsRoute::Shortcuts => self.settings_shortcuts_page(view.clone()),
                SettingsRoute::Plugins => self.settings_plugins_content(view.clone()),
                SettingsRoute::Engines | SettingsRoute::Services => {
                    self.settings_connections_content(content_width, view.clone())
                }
                SettingsRoute::Skills | SettingsRoute::Mcp => {
                    self.extensions_content(window, content_width, view.clone(), cx)
                }
                _ => {
                    let page = match route {
                        SettingsRoute::Appearance => {
                            self.settings_appearance_page(disabled, view.clone())
                        }
                        SettingsRoute::AutomaticNaming => {
                            self.settings_session_page(disabled, view.clone())
                        }
                        SettingsRoute::Git => self.settings_git_page(disabled, view.clone()),
                        SettingsRoute::Maintenance => {
                            self.settings_maintenance_page(disabled, view.clone())
                        }
                        SettingsRoute::Sessions => {
                            self.settings_conversation_page(disabled, view.clone())
                        }
                        _ => self.settings_general_page(disabled, view.clone()),
                    };
                    let controls = settings_controls(
                        format!(
                            "conduit-settings-page-{}-{:?}",
                            route.index(),
                            self.settings_ui.group
                        ),
                        page.into_page(self.settings_ui.group, view.clone()),
                    );
                    SettingsContent::new(controls)
                }
            }
        };
        h_flex()
            .id("settings-workspace-body")
            .debug_selector(|| "settings-workspace-body".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .items_start()
            .when(!compact, |body| {
                body.child(
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
                                .px_3()
                                .pb_3()
                                .overflow_y_scroll()
                                .child(self.settings_navigation(route, searching, view.clone())),
                        )
                        .child(
                            div()
                                .px_4()
                                .py_3()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(format!("Conduit {}", env!("CARGO_PKG_VERSION"))),
                        ),
                )
            })
            .child(self.settings_content_pane(compact, content, view))
    }

    pub(super) fn select_settings_page(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(route) = SettingsRoute::from_index(index) else {
            return;
        };
        if self.settings_ui.page != index {
            self.extensions_ui.cancel_mcp_detection();
        }
        self.settings_ui.page = route.index();
        self.settings_ui.group = None;
        self.settings_ui
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if route.extensions() {
            self.extensions_ui.mcp = route == SettingsRoute::Mcp;
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

    pub(super) fn generation_disclosure(
        &self,
        naming: bool,
        view: Entity<Self>,
        fields: Vec<SettingItem>,
    ) -> SettingItem {
        let expanded = if naming {
            self.settings_ui.naming_expanded
        } else {
            self.settings_ui.git_expanded
        };
        let summary = self.generation_summary(naming);
        SettingItem::render(move |options, window, cx| {
            let view = view.clone();
            settings_accordion(
                if naming {
                    "naming-config-expand"
                } else {
                    "git-config-expand"
                },
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(conduit::i18n::text_static(
                        "settings.navigation.configure_model",
                    ))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(summary.clone()),
                    ),
                expanded,
                accordion_fields(&fields, options, window, cx),
                move |open, _, cx| {
                    view.update(cx, |app, cx| {
                        if naming {
                            app.settings_ui.naming_expanded = open;
                        } else {
                            app.settings_ui.git_expanded = open;
                        }
                        cx.notify();
                    })
                },
            )
        })
        .keywords([
            conduit::i18n::text_static("model_picker.model_98fd0c"),
            conduit::i18n::text_static("model_picker.engine"),
            "Provider",
        ])
    }
}
