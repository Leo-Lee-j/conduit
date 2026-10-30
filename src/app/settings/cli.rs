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
            self.settings_provider_group(Provider::Claude, 0, disabled, view.clone()),
            self.settings_provider_group(Provider::Codex, 1, disabled, view.clone()),
            self.settings_provider_group(Provider::OpenCode, 2, disabled, view),
            settings_section("路径查找顺序")
                .items(vec![
                    value_item(
                        "环境变量",
                        "CONDUIT_CLAUDE_PATH · CONDUIT_CODEX_PATH · CONDUIT_OPENCODE_PATH".into(),
                    ),
                    value_item("应用配置", "在各引擎分组中手动指定".into()),
                    value_item("PATH", "claude / codex / opencode 的 .exe 或 .cmd".into()),
                    value_item(
                        "常见安装位置",
                        "~/.local/bin · %LOCALAPPDATA%\\Programs\\OpenAI\\Codex".into(),
                    ),
                ])
                .footer(|_, _| "按上面的顺序查找，前者优先。"),
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
            SettingItem::new("实际路径", paragraph_field(effective))
                .layout(Axis::Vertical)
                .description("按当前配置解析的路径，更换后对新启动的会话生效。"),
            SettingItem::new("状态", paragraph_field(diagnostic)).layout(Axis::Vertical),
            SettingItem::new("操作", actions).layout(Axis::Vertical),
        ];

        settings_section(provider.label())
            .items(capabilities)
            .footer(move |_, _| cli_footer.clone())
    }
}
