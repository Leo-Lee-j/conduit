//! Grouped navigation around the existing setting controls.
use super::*;

pub(in crate::app) struct SettingsUi {
    pub page: usize,
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
        "AI",
        "AI 引擎",
        "CLI Claude Codex OpenCode 路径 安装 登录 版本 PATH 环境变量",
    ),
    ("AI", "Skills", "skill 技能 插件 导入 编辑"),
    ("辅助任务", "自动命名", "会话 标题 模型 触发"),
    ("辅助任务", "Git 提交", "提交 提示词 模型"),
    ("维护", "数据与更新", "目录 配置文件 下载 重启 诊断 性能"),
    (
        "AI",
        "模型与服务",
        "API Key Base URL Provider CCS cc-switch 凭据 端点 连接",
    ),
    ("AI", "MCP 服务", "MCP 工具 配置 服务器 添加 编辑"),
    ("AI", "新会话默认", "默认 模型 服务 引擎 新建 对话"),
];

const PAGE_ORDER: [usize; 11] = [0, 1, 2, 3, 8, 10, 4, 9, 5, 6, 7];

fn matches_search(text: &str, query: &str) -> bool {
    let text = text.to_lowercase();
    query.split_whitespace().all(|word| text.contains(word))
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

impl ConduitApp {
    pub(super) fn settings_body(
        &self,
        window: &Window,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let query = self.settings_ui.search.read(cx).value().to_lowercase();
        let mut navigation = v_flex().gap_1().w_full();
        let mut group = "";
        for index in PAGE_ORDER {
            let (section, title, _) = &PAGES[index];
            if group != *section {
                group = section;
                navigation = navigation.child(
                    div()
                        .pt_2()
                        .pb_1()
                        .px_2()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(*section),
                );
            }
            let select = view.clone();
            navigation = navigation.child(
                Button::new(("settings-navigation", index))
                    .debug_selector(move || format!("settings-nav-{index}"))
                    .accessibility_label(*title)
                    .child(h_flex().w_full().justify_start().child(*title))
                    .small()
                    .ghost()
                    .w_full()
                    .justify_start()
                    .selected(self.settings_ui.page == index)
                    .cursor_pointer()
                    .on_click(move |_, window, cx| {
                        select.update(cx, |app, cx| {
                            app.select_settings_page(index, window, cx);
                        });
                    }),
            );
        }
        let content = if !query.trim().is_empty() {
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
            // The public library does not expose standalone page rendering. Hide
            // its redundant sidebar with a zero-width panel; controls keep their
            // native searchable fields and per-page scroll state.
            Settings::new(format!("conduit-settings-page-{}", self.settings_ui.page))
                .sidebar_width(px(0.))
                .sidebar_size_range(px(0.)..px(0.))
                .with_group_variant(GroupBoxVariant::Outline)
                .default_selected_index(gpui_kit::component::setting::SelectIndex {
                    page_ix: self.settings_ui.page,
                    group_ix: None,
                })
                .pages((0..PAGES.len()).map(|index| {
                    if index == self.settings_ui.page {
                        page.clone()
                    } else {
                        SettingPage::new(PAGES[index].1)
                    }
                }))
                .into_any_element()
        };
        h_flex()
            .size_full()
            .items_start()
            .child(
                v_flex()
                    .w(px(180.))
                    .h_full()
                    .flex_shrink_0()
                    .bg(rgb(palette().sidebar_bg))
                    .border_r_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div().p_3().child(
                            Input::new(&self.settings_ui.search)
                                .small()
                                .prefix(FullIconName::Search),
                        ),
                    )
                    .child(
                        div()
                            .id("settings-navigation-scroll")
                            .flex_1()
                            .min_h_0()
                            .w_full()
                            .p_2()
                            .overflow_y_scroll()
                            .child(navigation),
                    ),
            )
            .child(div().flex_1().min_w_0().h_full().child(content))
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

    fn settings_search_results(&self, query: &str, view: Entity<Self>) -> AnyElement {
        let mut results: Vec<(usize, String, String, Option<Provider>)> = PAGES
            .iter()
            .enumerate()
            .filter(|(_, (section, title, keywords))| {
                matches_search(&format!("{section} {title} {keywords}"), query)
            })
            .map(|(index, (section, title, keywords))| {
                (
                    index,
                    format!("{section} / {title}"),
                    format!("打开{title}设置 · {keywords}"),
                    None,
                )
            })
            .collect();
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
                results.push((
                    8,
                    provider.name.clone(),
                    format!("{} · 模型服务", provider.engine.label()),
                    Some(provider.engine),
                ));
            }
        }
        let count = results.len();
        let mut body = v_flex().gap_3().child(
            div()
                .text_size(px(18.))
                .font_semibold()
                .child(format!("搜索结果 · {count}")),
        );
        for (i, (page, title, detail, engine)) in results.into_iter().enumerate() {
            let view = view.clone();
            body = body.child(
                Button::new(("settings-search-result", i))
                    .debug_selector(move || format!("settings-search-result-{i}"))
                    .outline()
                    .w_full()
                    .h_auto()
                    .p_3()
                    .justify_start()
                    .child(
                        v_flex().gap_1().child(title).child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child(detail),
                        ),
                    )
                    .on_click(move |_, window, cx| {
                        view.update(cx, |app, cx| {
                            if let Some(engine) = engine {
                                app.connection_settings.engine = engine;
                            }
                            app.settings_ui.naming_expanded = true;
                            app.settings_ui.git_expanded = true;
                            app.settings_ui.prompt_expanded = true;
                            app.connection_settings.models_expanded = true;
                            app.select_settings_page(page, window, cx);
                        })
                    }),
            );
        }
        if count == 0 {
            body = body.child("没有匹配的设置。试试“API Key”“字体”“默认模型”或服务名称。");
        }
        div()
            .id("settings-search-results")
            .size_full()
            .overflow_y_scroll()
            .p_4()
            .child(body)
            .into_any_element()
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
