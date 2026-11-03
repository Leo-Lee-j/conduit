//! Grouped navigation around the existing setting controls.
use super::*;

pub(in crate::app) struct SettingsUi {
    pub page: usize,
    pub search: Entity<InputState>,
    pub naming_expanded: bool,
    pub git_expanded: bool,
    pub prompt_expanded: bool,
}

pub(super) const PAGES: [(&str, &str, &str); 8] = [
    ("工作台", "通用", "布局 内容 会话数量 标签"),
    ("工作台", "外观", "主题 字体 文字 代码"),
    ("工作台", "快捷键", "键盘 shortcut"),
    (
        "AI",
        "CLI & 模型服务",
        "Claude Codex OpenCode 路径 版本 Provider CCS",
    ),
    ("AI", "扩展", "Skills MCP skill 插件 工具"),
    ("AI", "会话", "自动 命名 模型 触发"),
    ("AI", "Git", "提交 提示词 模型"),
    ("维护", "数据与更新", "目录 配置文件 下载 重启 诊断 性能"),
];

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
        for (index, (section, title, keywords)) in PAGES.iter().enumerate() {
            if !format!("{section} {title} {keywords}")
                .to_lowercase()
                .contains(query.as_str())
            {
                continue;
            }
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
        let content = if self.settings_ui.page == 4 {
            self.extensions_content(window, view.clone(), cx)
        } else {
            let page = match self.settings_ui.page {
                1 => self.settings_appearance_page(disabled, view.clone()),
                2 => self.settings_shortcuts_page(),
                3 => self.settings_connections_page(disabled, view.clone()),
                5 => self.settings_session_page(disabled, view.clone()),
                6 => self.settings_git_page(disabled, view.clone()),
                7 => self.settings_maintenance_page(disabled, view.clone()),
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
                    .w(px(170.))
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
        self.settings_ui.page = index;
        if index == 4 {
            self.ensure_extensions(window, cx, false);
        } else {
            self.extensions_ui.probe = None;
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
