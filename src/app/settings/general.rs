//! General settings.

use super::*;

impl ConduitApp {
    pub(in crate::app) fn settings_general_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
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
        ])
    }

    pub(in crate::app) fn settings_maintenance_page(
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
        let auto_check = switch_field(
            view.clone(),
            |app| app.store.updates.auto_check,
            |app, value| app.store.updates.auto_check = value,
        );
        let auto_download = switch_field(
            view.clone(),
            |app| app.store.updates.auto_download,
            |app, value| app.store.updates.auto_download = value,
        );
        let auto_install = switch_field(
            view.clone(),
            |app| app.store.updates.auto_install,
            |app, value| app.store.updates.auto_install = value,
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
        settings_page("数据与更新", FullIconName::Settings).groups(vec![
            settings_section("本机数据")
                .items(vec![
                    value_item("数据目录", data_dir),
                    value_item("配置文件", state_file),
                ])
                .footer(|_, _| "配置只保存在本机。"),
            settings_section("应用更新").items(vec![
                SettingItem::new("自动检查更新", auto_check)
                    .description("启动后及每 6 小时后台检查")
                    .disabled(disabled),
                SettingItem::new("后台下载更新", auto_download)
                    .description("发现新版本后下载并验证")
                    .disabled(disabled),
                SettingItem::new("空闲时安装并重启", auto_install)
                    .description("任务结束后保存草稿，安装并重新打开应用")
                    .disabled(disabled),
            ]),
            settings_section("诊断").item(
                SettingItem::new("性能监测", performance)
                    .description("在底部工具栏显示性能入口")
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
        let mut items = vec![
            SettingItem::new("自动命名", auto_rename)
                .description("关闭后仍可手动使用 AI 重命名")
                .disabled(disabled),
            SettingItem::new("触发方式", mode).disabled(disabled),
            self.generation_disclosure(true, view.clone()),
        ];
        if self.settings_ui.naming_expanded {
            items.extend([
                SettingItem::new("命名引擎", provider).disabled(disabled),
                SettingItem::new("命名服务", channel).disabled(disabled),
                SettingItem::new("命名模型", model).disabled(disabled),
            ]);
        }
        settings_page("会话", FullIconName::Sparkles).group(
            settings_section("会话命名")
                .items(items)
                .footer(|_, _| "命名在后台进行；失败保留当前标题，手动命名不会被覆盖。"),
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
        let mut generation = vec![self.generation_disclosure(false, view.clone())];
        if self.settings_ui.git_expanded {
            generation.extend([
                SettingItem::new("生成引擎", provider).disabled(disabled),
                SettingItem::new("生成服务", channel).disabled(disabled),
                SettingItem::new("生成模型", model).disabled(disabled),
            ]);
        }
        let tag_settings_view = view.clone();
        let tag_increment = self.store.git.tag_increment;
        let tag_strategy = SettingField::render(move |_, _, _| {
            let app = tag_settings_view.clone();
            Button::new("git-tag-strategy")
                .small()
                .label(match tag_increment {
                    crate::store::TagIncrement::Patch => "Patch +1",
                    crate::store::TagIncrement::Minor => "Minor +1",
                    crate::store::TagIncrement::Major => "Major +1",
                })
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for (strategy, label) in [
                        (crate::store::TagIncrement::Patch, "Patch +1"),
                        (crate::store::TagIncrement::Minor, "Minor +1"),
                        (crate::store::TagIncrement::Major, "Major +1"),
                    ] {
                        let app = app.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label)
                                .checked(strategy == tag_increment)
                                .on_click(move |_, _, cx| {
                                    app.update(cx, |this, cx| {
                                        if this.writable && !this.closing {
                                            this.store.git.tag_increment = strategy;
                                            this.persist();
                                            cx.notify();
                                        }
                                    });
                                }),
                        );
                    }
                    menu
                })
        });
        let tag_step = count_stepper_field(
            "git-tag-step",
            self.store.git.tag_step.clamp(1, 100),
            1,
            100,
            "减少标签递增步长",
            "增加标签递增步长",
            disabled,
            view.clone(),
            |app, value| app.store.git.tag_step = value,
        );
        let expanded = self.settings_ui.prompt_expanded;
        let preview = if self.store.git.commit_message_prompt.trim().is_empty() {
            crate::store::DEFAULT_COMMIT_PROMPT
        } else {
            &self.store.git.commit_message_prompt
        }
        .to_owned();
        let edit = SettingItem::render(move |_, _, _| {
            let view = view.clone();
            v_flex()
                .w_full()
                .gap_2()
                .when(!expanded, |row| {
                    row.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(preview.clone()),
                    )
                })
                .child(
                    Button::new("git-prompt-expand")
                        .debug_selector(|| "git-prompt-expand".into())
                        .small()
                        .ghost()
                        .label(if expanded {
                            "收起提示词"
                        } else {
                            "编辑提示词"
                        })
                        .on_click(move |_, _, cx| {
                            view.update(cx, |app, cx| {
                                app.settings_ui.prompt_expanded = !expanded;
                                cx.notify();
                            });
                        }),
                )
        })
        .keywords(["提示词", "prompt"]);
        let mut prompts = vec![edit];
        if expanded {
            prompts.extend([
                SettingItem::new("提示词", prompt)
                    .layout(Axis::Vertical)
                    .disabled(disabled),
                SettingItem::new("恢复默认", reset).disabled(disabled),
            ]);
        }
        settings_page("Git", FullIconName::GitFork).groups([
            settings_section("标签生成").items(vec![
                SettingItem::new("版本递增策略", tag_strategy)
                    .description("根据已知稳定版标签生成新版本")
                    .disabled(disabled),
                SettingItem::new("递增步长", tag_step)
                    .description("目标段增加指定数值，右侧版本段归零")
                    .disabled(disabled),
            ]),
            settings_section("生成模型").items(generation),
            settings_section("Commit Message 提示词")
                .items(prompts)
                .footer(|_, _| "修改自动保存，下次生成生效；已暂存文件名和 diff 会自动附加。"),
        ])
    }
}
