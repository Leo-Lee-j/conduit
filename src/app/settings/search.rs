//! Searchable user tasks with destinations in the retained settings pages.
use super::navigation::{PAGES, matches_search};
use super::*;

#[derive(Clone)]
pub(in crate::app) struct SettingsResult {
    pub page: usize,
    pub group: Option<usize>,
    pub title: String,
    pub description: String,
    pub engine: Option<Provider>,
}

// Group indexes follow each page's visible information architecture. These
// targets open the section, including controls hidden behind disclosures.
const ENTRIES: &[(usize, usize, &str, &str, &str)] = &[
    (
        10,
        1,
        "默认会话权限",
        "新会话使用的默认权限",
        "权限 默认权限 默认会话权限 会话 工作区 permission default session workspace",
    ),
    (
        1,
        0,
        "自动将活跃工作空间排到前面",
        "优先显示有运行中会话的工作空间，再按最近会话活动时间排序。关闭后恢复手动顺序。",
        "工作区 侧栏 自动 排序 活跃 最近 workspace sidebar automatic sort active recent",
    ),
    (
        0,
        1,
        "应用语言",
        "选择界面语言，切换后立即生效。",
        "language locale English 简体中文 中文 英文 语言",
    ),
    (
        0,
        0,
        "内容宽度",
        "让聊天内容铺满窗口或居中显示",
        "内容铺满 布局 width",
    ),
    (
        10,
        2,
        "自动折叠问答记录",
        "开启后，已完成的问答记录默认折叠；关闭后默认展开。可逐条点击切换，已手动切换的状态不受影响。",
        "问答 问题 回答 选项 自动 折叠 question answer history automatic collapse",
    ),
    (
        0,
        0,
        "侧栏会话数量",
        "调整每个工作区默认显示的会话数",
        "列表 数量 sessions",
    ),
    (
        0,
        0,
        "标签页上限",
        "控制同时保留的标签页数量",
        "tab 标签 限制",
    ),
    (
        1,
        0,
        "应用主题",
        "选择浅色、深色或跟随系统",
        "外观 theme dark light",
    ),
    (
        1,
        1,
        "应用界面字体与字号",
        "调整侧栏、标签页、菜单、按钮与设置的字体和文字大小",
        "应用 软件 界面 UI 字体 字号 大小 界面字号 字体大小 文字大小 font size 侧栏 标签页 菜单 按钮",
    ),
    (
        1,
        1,
        "聊天字体与字号",
        "调整正文字体、字号和行高",
        "文字 字体 字号 行高 font size",
    ),
    (
        1,
        0,
        "自动折叠非活跃工作空间",
        "自动折叠最近 3 天内没有会话活动的工作空间。当前工作空间和正在运行的会话保持可见，仍可手动展开任意工作空间。",
        "工作区 侧栏 自动 折叠 非活跃 3天 workspace sidebar automatic collapse inactive",
    ),
    (
        1,
        1,
        "代码块显示",
        "调整代码字体、字号、行号、语法高亮与复制按钮",
        "代码 字体 行号 高亮 复制 code",
    ),
    (
        1,
        1,
        "文字预览与恢复默认",
        "预览界面、聊天和代码的文字效果",
        "重置 reset 预览",
    ),
    (
        2,
        0,
        "键盘快捷键",
        "查看并修改快捷键绑定，检查冲突或恢复默认",
        "keyboard shortcut 快捷键 按键 绑定 冲突 修改 自定义",
    ),
    (
        3,
        0,
        "CLI 路径、版本与更新",
        "启用或停用引擎，检测版本、检查官方更新并管理启动路径",
        "cli 安装 登录 版本 更新 检查 一键更新 升级 进度 upgrade latest update path Claude Codex OpenCode 启用 停用 能力",
    ),
    (
        8,
        0,
        "模型服务与 API Key",
        "添加或编辑服务地址、凭据与模型",
        "服务 Base URL 端点 密钥 api key provider",
    ),
    (
        8,
        1,
        "管理 CLI 模型",
        "添加或编辑模型，隐藏目录项，设置新会话默认",
        "列表 模型 catalog 添加 编辑 隐藏 恢复 默认",
    ),
    (
        10,
        2,
        "聊天显示偏好",
        "设置思考过程和已完成问答的默认折叠方式",
        "思考 推理 reasoning thinking question answer history 折叠 显示 collapse display",
    ),
    (
        10,
        3,
        "自动命名与触发方式",
        "决定何时生成会话标题",
        "重命名 标题 自动 触发 rename",
    ),
    (
        10,
        3,
        "命名使用的模型",
        "为标题生成单独选择引擎、服务和模型",
        "命名 模型 引擎 服务",
    ),
    (
        6,
        0,
        "Git 标签版本递增",
        "设置 Patch、Minor、Major 与递增步长",
        "git tag 标签 版本 策略 步长",
    ),
    (
        6,
        1,
        "提交说明生成模型",
        "为 Git 提交说明选择引擎、服务与模型",
        "git commit 模型 提交",
    ),
    (
        6,
        2,
        "提交说明提示词",
        "编辑生成提交说明时使用的指令",
        "git commit prompt 提示词",
    ),
    (
        7,
        0,
        "本机数据目录",
        "查看配置文件与数据的保存位置",
        "数据 目录 路径 本地 storage",
    ),
    (
        7,
        1,
        "自动更新",
        "控制更新检查、后台下载与空闲安装",
        "更新 下载 安装 重启 update",
    ),
    (
        7,
        2,
        "性能监测",
        "在工具栏中显示性能诊断入口",
        "性能 监测 诊断 performance",
    ),
];

impl ConduitApp {
    pub(in crate::app) fn settings_search_matches(&self, query: &str) -> Vec<SettingsResult> {
        if query.trim().is_empty() {
            return vec![];
        }
        let mut results: Vec<_> = ENTRIES
            .iter()
            .filter(|(page, _, title, description, keywords)| {
                matches_search(
                    &search_text(&[PAGES[*page].1, title, description, keywords]),
                    query,
                )
            })
            .map(|(page, group, title, description, _)| SettingsResult {
                page: *page,
                group: Some(*group),
                title: conduit::i18n::source(title).into(),
                description: conduit::i18n::source(description).into(),
                engine: None,
            })
            .collect();
        let notification_title =
            conduit::i18n::text_static("settings.general.system_notifications");
        let notification_description =
            conduit::i18n::text_static("settings.general.system_notifications_description");
        let mut notification_search = format!(
            "{notification_title} {notification_description} notifications system toast native Windows"
        );
        for language in conduit::i18n::languages() {
            let locale = language.id.clone().into();
            for key in [
                "settings.general.system_notifications",
                "settings.general.system_notifications_description",
            ] {
                notification_search.push(' ');
                notification_search.push_str(&conduit::i18n::text_for(&locale, key));
            }
        }
        if matches_search(&notification_search, query) {
            results.push(SettingsResult {
                page: 0,
                group: Some(0),
                title: notification_title.into(),
                description: notification_description.into(),
                engine: None,
            });
        }
        let fetch_title = conduit::i18n::text_static("settings.general.auto_fetch_incoming");
        let fetch_description =
            conduit::i18n::text_static("settings.general.auto_fetch_incoming_description");
        if matches_search(
            &format!("{fetch_title} {fetch_description} git fetch upstream 远端 上游"),
            query,
        ) {
            results.push(SettingsResult {
                page: 6,
                group: Some(3),
                title: fetch_title.into(),
                description: fetch_description.into(),
                engine: None,
            });
        }
        let background_title = conduit::i18n::text_static("wallpaper.title");
        let background_description = conduit::i18n::text_static("wallpaper.description");
        let mut background_search = String::from("Wallhaven wallpaper background");
        for language in conduit::i18n::languages() {
            let locale = language.id.clone().into();
            for key in [
                "wallpaper.title",
                "wallpaper.description",
                "wallpaper.color",
                "wallpaper.solid_only",
                "wallpaper.history",
                "wallpaper.choose_local",
                "wallpaper.resolution",
                "wallpaper.ratio",
                "wallpaper.top_range",
            ] {
                background_search.push(' ');
                background_search.push_str(&conduit::i18n::text_for(&locale, key));
            }
        }
        if matches_search(&background_search, query) {
            results.push(SettingsResult {
                page: 1,
                group: Some(2),
                title: background_title.into(),
                description: background_description.into(),
                engine: None,
            });
        }
        // Engine identity is explicit, independent of localized display labels.
        for engine in Provider::ALL {
            let title = conduit::i18n::format(
                "settings.search.engine_session_defaults",
                &[("engine", engine.label().into())],
            );
            let description = conduit::i18n::format(
                "settings.search.engine_session_defaults_description",
                &[("engine", engine.label().into())],
            );
            let mut aliases = format!(
                "{} 默认模型 新会话默认 默认服务 模型 服务 default model session defaults",
                engine.label()
            );
            for language in conduit::i18n::languages() {
                let locale = language.id.clone().into();
                for key in [
                    "settings.search.engine_session_defaults",
                    "settings.search.engine_session_defaults_description",
                ] {
                    aliases.push(' ');
                    aliases.push_str(&conduit::i18n::format_for(
                        &locale,
                        key,
                        &[("engine", engine.label().into())],
                    ));
                }
            }
            if matches_search(&aliases, query) {
                results.push(SettingsResult {
                    page: 10,
                    group: Some(0),
                    title,
                    description,
                    engine: Some(engine),
                });
            }
        }
        for (page, (section, title, keywords)) in PAGES.iter().enumerate() {
            // Naming is discoverable under Sessions; do not duplicate its legacy page.
            if page == navigation::SettingsRoute::AutomaticNaming.index() {
                continue;
            }
            if !results.iter().any(|result| result.page == page)
                && matches_search(&search_text(&[section, title, keywords]), query)
            {
                results.push(SettingsResult {
                    page,
                    group: None,
                    title: conduit::i18n::source(title).into(),
                    description: conduit::i18n::format(
                        "settings.search.view_all_settings",
                        &[("title", conduit::i18n::source(title).to_string())],
                    ),
                    engine: None,
                });
            }
        }
        for provider in &self.store.providers {
            if matches_search(
                &format!(
                    "{} {} {}",
                    provider.name,
                    provider.base_url,
                    provider.models.join(" ")
                ),
                query,
            ) {
                results.push(SettingsResult {
                    page: 8,
                    group: Some(0),
                    title: provider.name.clone(),
                    description: conduit::i18n::format(
                        "settings.search.model_service",
                        &[("arg0", provider.engine.label().to_string())],
                    ),
                    engine: Some(provider.engine),
                });
            }
        }
        for engine in Provider::ALL {
            for model in self
                .managed_native_models(engine, true)
                .into_iter()
                .filter(|model| {
                    matches_search(
                        &format!("{} {} {}", model.id, model.label, model.note),
                        query,
                    )
                })
                .take(20)
            {
                results.push(SettingsResult {
                    page: 8,
                    group: Some(if engine.descriptor().capabilities.model_services {
                        1
                    } else {
                        0
                    }),
                    title: model.label,
                    description: conduit::i18n::format(
                        "settings.search.cli_model",
                        &[
                            ("arg0", engine.label().to_string()),
                            ("arg1", model.id.to_string()),
                        ],
                    ),
                    engine: Some(engine),
                });
            }
        }
        results
    }

    pub(super) fn open_settings_result(
        &mut self,
        result: SettingsResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(engine) = result.engine {
            self.connection_settings.engine = engine;
        }
        // Old naming search targets still resolve to the consolidated section.
        let page = if result.page == 5 { 10 } else { result.page };
        let target_group = if result.page == 5 {
            Some(3)
        } else {
            result.group
        };
        // OpenCode manages services in its native configuration; do not link
        // to a group that is absent in that engine's shorter page.
        let group = if page == 8
            && target_group.is_some()
            && self.connection_settings.engine == Provider::OpenCode
        {
            Some(0)
        } else {
            target_group
        };
        self.settings_ui.naming_expanded |= page == 10 && group == Some(3);
        self.settings_ui.git_expanded |= result.page == 6;
        self.settings_ui.prompt_expanded |= result.page == 6 && group == Some(2);
        self.connection_settings.models_expanded |= result.page == 8;
        self.select_settings_page(page, window, cx);
        self.settings_ui.group = group;
        self.settings_ui.focus.focus(window, cx);
    }

    pub(super) fn settings_search_results(&self, query: &str, view: Entity<Self>) -> AnyElement {
        let results = self.settings_search_matches(query);
        let count = results.len();
        let clear = view.clone();
        let mut body = v_flex().w_full().gap_3().child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_size(ui_text_size(22.)).font_semibold().child(
                            conduit::i18n::text_static("settings.search.search_settings"),
                        ))
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::format(
                                    "settings.search.results_open_a_group_or_press_enter",
                                    &[("count", format!("{}", count))],
                                )),
                        ),
                )
                .child(
                    Button::new("settings-search-clear")
                        .ghost()
                        .label(conduit::i18n::text_static("settings.search.clear_search"))
                        .on_click(move |_, window, cx| {
                            clear.update(cx, |app, cx| {
                                app.settings_ui
                                    .search
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                app.settings_ui
                                    .search
                                    .read(cx)
                                    .focus_handle(cx)
                                    .focus(window, cx);
                                cx.notify();
                            })
                        }),
                ),
        );
        for (index, result) in results.into_iter().enumerate() {
            let select = view.clone();
            let (section, page, _) = PAGES[result.page];
            body = body.child(
                Button::new(("settings-search-result", index))
                    .debug_selector(move || format!("settings-search-result-{index}"))
                    .accessibility_label(format!(
                        "{} / {}: {}",
                        conduit::i18n::source(section),
                        conduit::i18n::source(page),
                        result.title
                    ))
                    .outline()
                    .w_full()
                    .h_auto()
                    .p_4()
                    .justify_start()
                    .child(
                        v_flex()
                            .min_w_0()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "{} / {}",
                                        conduit::i18n::source(section),
                                        conduit::i18n::source(page)
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_semibold()
                                    .child(result.title.clone()),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(13.))
                                    .text_color(rgb(palette().muted))
                                    .child(result.description.clone()),
                            ),
                    )
                    .on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| {
                            app.open_settings_result(result.clone(), window, cx)
                        })
                    }),
            );
        }
        if count == 0 {
            body = body.child(
                v_flex()
                    .py_6()
                    .gap_3()
                    .child(conduit::i18n::text_static(
                        "settings.search.no_matching_settings",
                    ))
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static(
                                "settings.search.try_a_shorter_keyword_or_browse_the",
                            )),
                    )
                    .child(
                        h_flex().gap_2().children(
                            [
                                conduit::i18n::text_static("settings.appearance.font"),
                                conduit::i18n::text_static("model_picker.default_model"),
                                "API Key",
                                conduit::i18n::text_static("settings.navigation.shortcuts"),
                            ]
                            .map(|word| {
                                let view = view.clone();
                                Button::new(format!("settings-search-suggestion-{word}"))
                                    .outline()
                                    .small()
                                    .label(word)
                                    .on_click(move |_, window, cx| {
                                        view.update(cx, |app, cx| {
                                            app.settings_ui.search.update(cx, |input, cx| {
                                                input.set_value(word, window, cx)
                                            });
                                            cx.notify();
                                        })
                                    })
                            }),
                        ),
                    ),
            );
        }
        div()
            .id("settings-search-results")
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .child(body)
            .into_any_element()
    }
}

/// Search all supported translations plus original aliases, independent of UI locale.
fn search_text(parts: &[&str]) -> String {
    let mut text = parts.join(" ");
    for language in conduit::i18n::languages() {
        let locale = language.id.clone().into();
        for part in parts {
            text.push(' ');
            text.push_str(conduit::i18n::source_for(&locale, part));
        }
    }
    text
}
