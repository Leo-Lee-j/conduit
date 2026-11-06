//! Grouped navigation around the existing setting controls.
use super::*;

pub(in crate::app) struct SettingsUi {
    pub open: bool,
    pub focus: FocusHandle,
    pub page: usize,
    pub group: Option<usize>,
    pub search: Entity<InputState>,
    pub naming_expanded: bool,
    pub git_expanded: bool,
    pub prompt_expanded: bool,
}

pub(super) const PAGES: [(&str, &str, &str); 11] = [
    ("工作台", "通用", "布局 内容 会话数量 标签"),
    ("工作台", "外观", "主题 字体 文字 代码"),
    ("工作台", "快捷键", "键盘 shortcut"),
    (
        "AI 连接",
        "AI 引擎",
        "CLI Claude Codex OpenCode 路径 安装 登录 版本 PATH 环境变量 启用 停用 能力",
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
];

const PAGE_ORDER: [usize; 11] = [0, 1, 2, 3, 8, 10, 4, 9, 5, 6, 7];

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
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let query = self.settings_ui.search.read(cx).value();
        let searching = !query.trim().is_empty();
        let mut navigation = v_flex().gap_1().w_full();
        let mut group = "";
        for index in PAGE_ORDER {
            let (section, title, _) = PAGES[index];
            if group != section {
                group = section;
                navigation = navigation.child(
                    div()
                        .pt_3()
                        .pb_1()
                        .px_3()
                        .text_size(px(12.))
                        .font_semibold()
                        .text_color(rgb(palette().muted))
                        .child(section),
                );
            }
            let select = view.clone();
            let icon = match index {
                1 => FullIconName::Palette,
                2 => FullIconName::Keyboard,
                3 => FullIconName::SquareTerminal,
                4 | 9 => FullIconName::Blocks,
                5 => FullIconName::Sparkles,
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
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .child(Icon::new(icon).small())
                            .child(title),
                    )
                    .ghost()
                    .h(px(34.))
                    .w_full()
                    .justify_start()
                    .text_size(px(13.))
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
        } else if matches!(self.settings_ui.page, 4 | 9) {
            self.extensions_content(window, view.clone(), cx)
        } else {
            let page = match self.settings_ui.page {
                1 => self.settings_appearance_page(disabled, view.clone()),
                2 => self.settings_shortcuts_page(),
                3 | 8 => self.settings_connections_page(disabled, view.clone()),
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
            .with_group_variant(GroupBoxVariant::Outline)
            .default_selected_index(gpui_kit::component::setting::SelectIndex {
                page_ix: 0,
                group_ix: None,
            })
            .page(page.into_page(self.settings_ui.group, view))
            .into_any_element()
        };
        h_flex()
            .size_full()
            .items_start()
            .child(
                v_flex()
                    .w(px(if f32::from(window.viewport_size().width) < 1000. {
                        184.
                    } else {
                        216.
                    }))
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(palette().sidebar_bg))
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
                            .child(navigation),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_3()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(format!("Conduit {}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(div().w_full().max_w(px(1040.)).h_full().child(content)),
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
            return "跟随当前会话".into();
        }
        let route = match channel {
            Some(ModelChannel::Provider(id)) => self
                .store
                .providers
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.as_str())
                .unwrap_or("服务不可用"),
            Some(ModelChannel::Ccs(_)) => "CCS",
            Some(ModelChannel::Cli) => "CLI 默认",
            None => {
                if provider.is_some() {
                    "CLI 默认"
                } else {
                    "跟随会话服务"
                }
            }
        };
        format!(
            "{} · {} · {}",
            provider.map(Provider::label).unwrap_or("跟随会话引擎"),
            route,
            model.as_deref().unwrap_or("默认模型")
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
                        .text_size(px(12.))
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
                        "收起配置"
                    } else {
                        "配置模型"
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
        .keywords(["模型", "引擎", "Provider"])
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, ModelProvider, Provider, Store};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton, TestAppContext, px};

    #[gpui_kit::test]
    fn search_opens_matching_service_engine_and_keeps_navigation_visible(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app =
                    ConduitApp::with_store(window, cx, Ok(Store::default()), false, false);
                app.writable = false;
                app.connection_settings.syncing = true;
                app.connection_settings.engine = Provider::Claude;
                app.store.providers = vec![ModelProvider {
                    id: "test".into(),
                    name: "Unique Proxy".into(),
                    engine: Provider::Codex,
                    base_url: "https://example.invalid/v1".into(),
                    api_key_env: String::new(),
                    protected_api_key: None,
                    models: vec!["model-x".into()],
                    source: Default::default(),
                    model_metadata: vec![],
                }];
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(gpui_kit::size(px(850.), px(500.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.open_settings(window, cx);
                app.settings_ui
                    .search
                    .update(cx, |input, cx| input.set_value("unique proxy", window, cx));
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("settings-nav-3").is_some());
        let result = cx.debug_bounds("settings-search-result-0").unwrap();
        cx.simulate_mouse_down(result.center(), MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(result.center(), MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(app.read(cx).settings_ui.page, 8);
            assert_eq!(app.read(cx).connection_settings.engine, Provider::Codex);
            assert!(app.read(cx).settings_ui.search.read(cx).value().is_empty());
        });
    }
}
