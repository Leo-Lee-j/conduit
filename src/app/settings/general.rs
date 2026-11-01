//! General settings.

use super::*;

impl ConduitApp {
    pub(in crate::app) fn settings_general_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let state_path = Store::path();
        let data_dir = state_path
            .parent()
            .map(display_path)
            .unwrap_or_else(|| "—".into());
        let state_file = display_path(&state_path);
        let sessions_limit = count_stepper_field(
            "sessions-limit",
            self.store.ui.sessions_per_workspace.clamp(1, 30),
            1,
            30,
            "减少工作区会话数",
            "增加工作区会话数",
            disabled,
            view.clone(),
            |app, value| app.store.ui.sessions_per_workspace = value,
        );
        let max_tabs_limit = count_stepper_field(
            "max-tabs",
            self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT),
            1,
            MAX_TAB_LIMIT,
            "减少最大标签数量",
            "增加最大标签数量",
            disabled,
            view.clone(),
            |app, value| app.store.ui.max_tabs = value,
        );
        let full_width = switch_field(
            view.clone(),
            |app| app.store.ui.full_width_content,
            |app, value| app.store.ui.full_width_content = value,
        );
        let performance_view = view.clone();
        let performance_read = performance_view.clone();
        let performance = SettingField::switch(
            move |cx: &App| performance_read.read(cx).store.ui.performance_monitor,
            move |value, cx| {
                performance_view.update(cx, |this, cx| {
                    if !this.writable || this.closing {
                        return;
                    }
                    this.store.ui.performance_monitor = value;
                    if value {
                        this.performance.update(cx, |monitor, _| monitor.reset());
                    } else {
                        this.performance_open = false;
                    }
                    this.persist();
                    cx.notify();
                });
            },
        );
        settings_page("通用", FullIconName::Settings).groups(vec![
            settings_section("工作台布局").items(vec![
                SettingItem::new("内容铺满", full_width)
                    .description("开启后聊天内容铺满可用区域，关闭时居中显示")
                    .disabled(disabled),
                SettingItem::new("侧栏会话数量", sessions_limit)
                    .description("每个工作区默认显示的会话数，更多会话可展开查看")
                    .disabled(disabled),
                SettingItem::new("标签页上限", max_tabs_limit)
                    .description("超出上限时自动关闭最早打开的标签页")
                    .disabled(disabled),
            ]),
            settings_section("本机数据")
                .items(vec![
                    value_item("数据目录", data_dir),
                    value_item("配置文件", state_file),
                ])
                .footer(|_, _| "配置只保存在本机；外观与诊断设置随本地状态保存。"),
            settings_section("诊断").item(
                SettingItem::new("性能监测", performance)
                    .description("在底部工具栏显示性能入口并采集指标")
                    .disabled(disabled),
            ),
        ])
    }

    pub(in crate::app) fn settings_session_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let rename_view = view.clone();
        let rename_read = rename_view.clone();
        let auto_rename = SettingField::switch(
            move |cx: &App| rename_read.read(cx).store.ui.auto_rename,
            move |value, cx| {
                rename_view.update(cx, |this, cx| {
                    if !this.writable || this.closing {
                        return;
                    }
                    this.store.ui.auto_rename = value;
                    if !value {
                        this.session_namings.retain(|_, naming| {
                            if naming.automatic {
                                naming.runtime.cancel();
                                false
                            } else {
                                true
                            }
                        });
                    }
                    this.persist();
                    cx.notify();
                });
            },
        );
        let provider_view = view.clone();
        let provider = SettingField::render(move |_, _, cx| {
            provider_view
                .read(cx)
                .naming_provider_picker(provider_view.clone(), disabled)
        });
        let channel_view = view.clone();
        let channel = SettingField::render(move |_, _, cx| {
            channel_view
                .read(cx)
                .naming_channel_picker(channel_view.clone(), disabled)
        });
        let model_view = view.clone();
        let model = SettingField::render(move |_, _, cx| {
            model_view
                .read(cx)
                .naming_model_picker(model_view.clone(), disabled)
        });
        let mode_view = view.clone();
        let mode = SettingField::render(move |_, _, cx| {
            let auto_rename = mode_view.read(cx).store.ui.auto_rename;
            mode_view
                .read(cx)
                .naming_mode_picker(mode_view.clone(), disabled || !auto_rename)
        });
        settings_page("会话", FullIconName::Sparkles).group(
            settings_section("会话命名")
                .items(vec![
                    SettingItem::new("自动命名", auto_rename)
                        .description(
                            "自动生成和更新会话标题；关闭后仍可在会话菜单手动使用 AI 重命名",
                        )
                        .disabled(disabled),
                    SettingItem::new("命名引擎", provider)
                        .description("生成会话标题使用的引擎；默认跟随当前会话")
                        .disabled(disabled),
                    SettingItem::new("命名 Provider", channel)
                        .description("独立选择 CLI 默认、CCS 或自定义 Provider；OpenCode 使用原生 Provider")
                        .disabled(disabled),
                    SettingItem::new("命名模型", model)
                        .description("模型列表随 Provider 切换；全部默认时跟随当前会话")
                        .disabled(disabled),
                    SettingItem::new("触发方式", mode)
                        .description("选择标题跟随最新对话自动更新，或只按第一条消息生成一次")
                        .disabled(disabled),
                ])
                .footer(|_, _| {
                    "命名在后台进行，失败时保留当前标题；命名完成前手动重命名的会话不会被覆盖。右键会话可随时使用 AI 重命名。"
                }),
        )
    }

    pub(in crate::app) fn settings_git_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let provider_view = view.clone();
        let provider = SettingField::render(move |_, _, cx| {
            provider_view
                .read(cx)
                .git_provider_picker(provider_view.clone(), disabled)
        });
        let channel_view = view.clone();
        let channel = SettingField::render(move |_, _, cx| {
            channel_view
                .read(cx)
                .git_channel_picker(channel_view.clone(), disabled)
        });
        let model_view = view.clone();
        let model = SettingField::render(move |_, _, cx| {
            model_view
                .read(cx)
                .git_model_picker(model_view.clone(), disabled)
        });
        let prompt_input = self.git_prompt_input.clone();
        let prompt = SettingField::render(move |_, _, _| {
            Textarea::new(&prompt_input)
                .h(px(180.))
                .w_full()
                .text_size(px(12.))
                .aria_label("Commit Message 生成提示词")
                .disabled(disabled)
        });
        let reset_view = view.clone();
        let reset_input = self.git_prompt_input.clone();
        let reset = SettingField::render(move |_, _window, _| {
            Button::new("git-prompt-reset")
                .label("恢复默认")
                .small()
                .ghost()
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click({
                    let view = reset_view.clone();
                    let input = reset_input.clone();
                    move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            this.store.git.commit_message_prompt =
                                crate::store::DEFAULT_COMMIT_PROMPT.into();
                            input.update(cx, |input, cx| {
                                input.set_value(crate::store::DEFAULT_COMMIT_PROMPT, window, cx);
                            });
                            this.persist();
                            cx.notify();
                        });
                    }
                })
        });
        settings_page("Git", FullIconName::GitFork).groups(vec![
            settings_section("生成引擎").items(vec![
                SettingItem::new("生成引擎", provider)
                    .description("用于生成 Commit Message 的 CLI；默认跟随当前会话")
                    .disabled(disabled),
                SettingItem::new("生成 Provider", channel)
                    .description("独立选择 CLI 默认、CCS 或自定义 Provider；OpenCode 使用原生 Provider")
                    .disabled(disabled),
                SettingItem::new("生成模型", model)
                    .description("模型列表随 Provider 切换；全部默认时跟随当前会话")
                    .disabled(disabled),
            ]),
            settings_section("Commit Message 提示词").items(vec![
                SettingItem::new("提示词", prompt)
                    .layout(Axis::Vertical)
                    .description(
                        "设置生成说明的语言、格式和风格，应用于 Git 侧栏的 Generate 按钮。已暂存文件名与 diff 会自动附加，无需填写占位符。",
                    )
                    .disabled(disabled),
                SettingItem::new("恢复默认", reset)
                    .description(
                        "自动保存，适用于所有工作区；留空时使用默认提示词。修改将在下一次生成时生效。",
                    )
                    .disabled(disabled),
            ]),
        ])
    }
}
