//! Searchable user tasks with destinations in the retained settings pages.
use super::navigation::{PAGES, matches_search};
use super::*;

#[derive(Clone)]
pub(super) struct SettingsResult {
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
        0,
        0,
        "内容宽度",
        "让聊天内容铺满窗口或居中显示",
        "内容铺满 布局 width",
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
        2,
        "聊天字体与字号",
        "调整正文字体、字号和行高",
        "文字 字体 字号 行高 font size",
    ),
    (
        1,
        3,
        "代码块显示",
        "调整代码字体、字号、行号、语法高亮与复制按钮",
        "代码 字体 行号 高亮 复制 code",
    ),
    (
        1,
        4,
        "文字预览与恢复默认",
        "预览聊天效果，恢复文字与代码的默认显示",
        "重置 reset 预览",
    ),
    (
        2,
        0,
        "键盘快捷键",
        "查看会话、文件与设置的快捷键",
        "keyboard shortcut 快捷键",
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
        0,
        "Claude Code 默认模型",
        "为新的 Claude Code 会话指定服务与模型",
        "新会话 默认 claude",
    ),
    (
        10,
        1,
        "Codex 默认模型",
        "为新的 Codex 会话指定服务与模型",
        "新会话 默认 codex",
    ),
    (
        10,
        2,
        "OpenCode 默认模型",
        "为新的 OpenCode 会话指定服务与模型",
        "新会话 默认 opencode",
    ),
    (
        5,
        0,
        "自动命名与触发方式",
        "决定何时生成会话标题",
        "重命名 标题 自动 触发 rename",
    ),
    (
        5,
        0,
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
    pub(super) fn settings_search_matches(&self, query: &str) -> Vec<SettingsResult> {
        if query.trim().is_empty() {
            return vec![];
        }
        let mut results: Vec<_> = ENTRIES
            .iter()
            .filter(|(page, _, title, description, keywords)| {
                matches_search(
                    &format!("{} {title} {description} {keywords}", PAGES[*page].1),
                    query,
                )
            })
            .map(|(page, group, title, description, _)| SettingsResult {
                page: *page,
                group: Some(*group),
                title: (*title).into(),
                description: (*description).into(),
                engine: None,
            })
            .collect();
        for (page, (section, title, keywords)) in PAGES.iter().enumerate() {
            if !results.iter().any(|result| result.page == page)
                && matches_search(&format!("{section} {title} {keywords}"), query)
            {
                results.push(SettingsResult {
                    page,
                    group: None,
                    title: (*title).into(),
                    description: format!("查看{title}的全部设置"),
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
                    description: format!("{} · 模型服务", provider.engine.label()),
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
                    description: format!("{} · CLI 模型 · {}", engine.label(), model.id),
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
        // OpenCode manages services in its native configuration; do not link
        // to a group that is absent in that engine's shorter page.
        let group = if result.page == 8
            && result.group.is_some()
            && self.connection_settings.engine == Provider::OpenCode
        {
            Some(0)
        } else {
            result.group
        };
        self.settings_ui.naming_expanded |= result.page == 5;
        self.settings_ui.git_expanded |= result.page == 6;
        self.settings_ui.prompt_expanded |= result.page == 6 && group == Some(2);
        self.connection_settings.models_expanded |= result.page == 8;
        self.select_settings_page(result.page, window, cx);
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
                        .child(
                            div()
                                .text_size(ui_text_size(22.))
                                .font_semibold()
                                .child("搜索设置"),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .text_color(rgb(palette().muted))
                                .child(format!(
                                    "找到 {count} 项结果 · 打开对应分组，或按 Enter 打开第一项"
                                )),
                        ),
                )
                .child(
                    Button::new("settings-search-clear")
                        .ghost()
                        .label("清除搜索")
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
                                    .child(format!("{section} / {page}")),
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
                    .child("没有找到匹配的设置")
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .text_color(rgb(palette().muted))
                            .child("试试更短的关键词，或从左侧分类浏览。"),
                    )
                    .child(h_flex().gap_2().children(
                        ["字体", "默认模型", "API Key", "快捷键"].map(|word| {
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
                    )),
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

#[cfg(test)]
mod tests {
    use super::matches_search;
    #[test]
    fn settings_search_matches_words_case_and_whitespace() {
        assert!(matches_search("模型服务 API Key", "  api KEY  "));
        assert!(matches_search("聊天字号 行高", "字号 行高"));
        assert!(!matches_search("模型服务 API Key", "api 字体"));
    }
}
