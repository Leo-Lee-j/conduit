//! Cli settings.

use super::*;

impl ConduitApp {
    pub(in crate::app) fn choose_cli(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(format!("选择 {} 可执行文件（.exe 或 .cmd）", provider.label()).into()),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        match cli::validate_executable(&path) {
                            Ok(path) => this.set_cli_path(provider, Some(path), cx),
                            Err(error) => {
                                this.diagnostics
                                    .insert(provider, format!("路径未保存：{error:#}"));
                            }
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.diagnostics
                        .insert(provider, format!("打开文件失败：{other:?}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(in crate::app) fn set_cli_path(
        &mut self,
        provider: Provider,
        path: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let previous = self.store.cli.clone();
        self.store.cli.set(provider, path);
        if self.persist() {
            if provider == Provider::OpenCode {
                self.opencode_model_probes.clear();
                self.opencode_catalogs.clear();
            }
            if let Some(probe) = self.probes.get(&provider) {
                probe.cancel();
            }
            self.diagnostics
                .insert(provider, "配置已保存，用于后续启动；请检测版本。".into());
        } else {
            self.store.cli = previous;
            self.diagnostics.insert(
                provider,
                self.message
                    .clone()
                    .unwrap_or_else(|| "配置保存失败".into()),
            );
        }
        cx.notify();
    }

    pub(in crate::app) fn probe_cli(&mut self, provider: Provider, cx: &mut Context<Self>) {
        if self.closing || self.probes.contains_key(&provider) {
            return;
        }
        match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => {
                self.probes
                    .insert(provider, VersionProbe::start(provider, cli.path));
                self.diagnostics
                    .insert(provider, "正在运行 --version，最多等待 10 秒…".into());
            }
            Err(error) => {
                self.diagnostics
                    .insert(provider, format!("无法检测：{error:#}"));
            }
        }
        cx.notify();
    }

    pub(in crate::app) fn settings_cli_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        settings_page("CLI", FullIconName::SquareTerminal).groups(vec![
            SettingGroup::new()
                .title("路径查找顺序")
                .items(vec![
                    value_item(
                        "环境变量",
                        "CONDUIT_CLAUDE_PATH · CONDUIT_CODEX_PATH · CONDUIT_OPENCODE_PATH".into(),
                    ),
                    value_item("应用配置", "在下面各引擎分组中手动指定".into()),
                    value_item("PATH", "claude / codex / opencode 的 .exe 或 .cmd".into()),
                    value_item(
                        "常见安装位置",
                        "~/.local/bin · %LOCALAPPDATA%\\Programs\\OpenAI\\Codex".into(),
                    ),
                ])
                .footer(|_, _| "按上面的顺序查找，前者优先。"),
            self.settings_provider_group(Provider::Claude, 0, disabled, view.clone()),
            self.settings_provider_group(Provider::Codex, 1, disabled, view.clone()),
            self.settings_provider_group(Provider::OpenCode, 2, disabled, view),
        ])
    }

    fn settings_provider_group(
        &self,
        provider: Provider,
        index: usize,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let closing = self.closing;
        let configured = self
            .store
            .cli
            .path(provider)
            .map(display_path)
            .unwrap_or_else(|| "自动查找（环境变量 / PATH）".into());
        let has_path = self.store.cli.path(provider).is_some();
        let effective = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => format!("{}：{}", cli.source.label(), display_path(&cli.path)),
            Err(error) => format!("尚不可用：{error:#}"),
        };
        let probing = self.probes.contains_key(&provider);
        let diagnostic = self
            .diagnostics
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| "尚未检测版本。".into());
        let tested = cli::tested_version(provider);
        let mut cli_footer = format!(
            "版本检测只运行 --version，不调用模型，也不验证登录状态；请先在官方 CLI 中完成登录。已验证版本 {tested}。"
        );
        if provider == Provider::OpenCode {
            cli_footer.push_str(
                " ACP 接入使用 OpenCode 自身登录与 Provider 配置；外部插件、问答工具与子代理暂未开放。",
            );
        }

        let choose_view = view.clone();
        let choose = SettingField::render(move |_, _, _| {
            let choose_view = choose_view.clone();
            Button::new(("settings-choose", index))
                .label("更改")
                .small()
                .outline()
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click(move |_, window, cx| {
                    choose_view.update(cx, |this, cx| {
                        this.choose_cli(provider, window, cx);
                    });
                })
        });

        let clear_view = view.clone();
        let probe_view = view.clone();
        let cancel_view = view.clone();
        let actions = SettingField::render(move |_, _, _| {
            let mut row = h_flex().w_full().justify_end().gap_2().child({
                let clear_view = clear_view.clone();
                Button::new(("settings-clear", index))
                    .label("清除配置")
                    .small()
                    .ghost()
                    .disabled(disabled || !has_path)
                    .when(!disabled && has_path, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        clear_view.update(cx, |this, cx| {
                            this.set_cli_path(provider, None, cx);
                        });
                    })
            });
            if probing {
                let cancel_view = cancel_view.clone();
                row = row.child(
                    Button::new(("settings-cancel-probe", index))
                        .label("取消检测")
                        .small()
                        .ghost()
                        .disabled(closing)
                        .when(!closing, |button| button.cursor_pointer())
                        .on_click(move |_, _, cx| {
                            cancel_view.update(cx, |this, cx| {
                                if let Some(probe) = this.probes.get(&provider) {
                                    probe.cancel();
                                }
                                this.diagnostics.insert(provider, "正在停止检测…".into());
                                cx.notify();
                            });
                        }),
                );
            } else {
                let probe_view = probe_view.clone();
                row = row.child(
                    Button::new(("settings-probe", index))
                        .label("检测版本")
                        .small()
                        .outline()
                        .disabled(closing)
                        .when(!closing, |button| button.cursor_pointer())
                        .on_click(move |_, _, cx| {
                            probe_view.update(cx, |this, cx| {
                                this.probe_cli(provider, cx);
                            });
                        }),
                );
            }
            row
        });

        let capabilities = vec![
            SettingItem::new("可执行文件", choose)
                .description(configured)
                .disabled(disabled),
            SettingItem::new("本次启动", paragraph_field(effective))
                .layout(Axis::Vertical)
                .description("当前进程实际使用的程序；更换后对下一次启动的会话生效。"),
            SettingItem::new("状态", paragraph_field(diagnostic)).layout(Axis::Vertical),
            SettingItem::new("操作", actions).layout(Axis::Vertical),
        ];

        let catalog = self.engine_catalog(provider);
        let current_model = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| "CLI 默认".into());
        let source = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let catalog_path = catalog
            .and_then(|catalog| catalog.catalog_path.as_deref())
            .map(display_path);
        let endpoint = catalog
            .and_then(|catalog| catalog.endpoint.clone())
            .unwrap_or_else(|| "CLI 内置".into());
        let detail = catalog.and_then(|catalog| catalog.detail.clone());
        let model_count = catalog.map(|catalog| catalog.models.len()).unwrap_or(0);
        let rows: Vec<_> = catalog
            .map(|catalog| {
                let current = catalog.current.as_deref();
                catalog
                    .models
                    .iter()
                    .map(|model| {
                        (
                            model.id.clone(),
                            model.label.clone(),
                            model.note.clone(),
                            current == Some(model.id.as_str()),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let empty = rows.is_empty();
        let catalog_error = catalog.and_then(|catalog| catalog.error.clone());
        let catalog_note = catalog.and_then(|catalog| catalog.note.clone());
        let mut model_items = vec![value_item("来源", source)];
        if let Some(catalog_path) = catalog_path {
            model_items.push(value_item("模型目录", catalog_path));
        }
        model_items.push(value_item("当前默认", current_model));
        model_items.push(value_item("接口", endpoint));
        if let Some(detail) = detail {
            model_items.push(value_item("附加配置", detail));
        }
        model_items.push(value_item("可用模型", format!("{model_count} 个")));
        if let Some(catalog_error) = catalog_error {
            model_items.push(
                SettingItem::new("读取失败", error_field(catalog_error))
                    .layout(Axis::Vertical)
                    .keywords(["错误"]),
            );
        }
        let list = SettingField::render(move |_, _, _| {
            let mut list = v_flex().w_full().gap_2();
            for (id, label, note, current) in &rows {
                let show_id = label != id;
                list = list.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(model_mark(provider, Some(id)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette().text))
                                        .child(label.clone()),
                                )
                                .when(show_id, |this| {
                                    this.child(
                                        div()
                                            .truncate()
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().faint))
                                            .child(id.clone()),
                                    )
                                }),
                        )
                        .when(!note.is_empty(), |this| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(note.clone()),
                            )
                        })
                        .when(*current, |this| {
                            this.child(
                                div()
                                    .px_2()
                                    .py(px(2.))
                                    .rounded(px(4.))
                                    .bg(rgb(palette().surface_hover))
                                    .text_size(px(9.))
                                    .text_color(rgb(palette().muted))
                                    .child("CLI 当前"),
                            )
                        }),
                );
            }
            if empty {
                list = list.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
            list
        });
        model_items.push(
            SettingItem::new("模型列表", list)
                .layout(Axis::Vertical)
                .description("来自 CLI 自身配置，只读；切换模型请在 CLI 中操作。"),
        );
        let sync_view = view.clone();
        let sync = SettingField::render(move |_, _, _| {
            let sync_view = sync_view.clone();
            Button::new(("settings-sync-models", index))
                .label("重新读取")
                .small()
                .ghost()
                .disabled(closing)
                .when(!closing, |button| button.cursor_pointer())
                .on_click(move |_, _, cx| {
                    sync_view.update(cx, |this, cx| {
                        this.reload_models(provider, cx);
                    });
                })
        });
        model_items.push(
            SettingItem::new("重新读取", sync)
                .description("重新读取 CLI 配置中的模型定义")
                .disabled(closing),
        );
        let model_footer = catalog_note.unwrap_or_else(|| {
            "此处只显示 CLI 当前模型；完整来源与 CCS 同步见「模型同步」页面。不读取或显示密钥，也不会写回 CLI 配置。".into()
        });
        let footer = format!("{cli_footer}\n\n{model_footer}");

        SettingGroup::new()
            .title(provider.label())
            .description("可执行文件、版本检测与 CLI 当前模型")
            .items(capabilities)
            .items(model_items)
            .footer(move |_, _| footer.clone())
    }
}
