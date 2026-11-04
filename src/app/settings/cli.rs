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
            self.extensions_ui.invalidate();
            if provider == Provider::OpenCode {
                self.opencode_model_probes.clear();
                self.opencode_catalogs.clear();
            }
            if let Some(probe) = self.probes.remove(&provider) {
                probe.cancel();
            }
            self.probe_cli(provider, cx);
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

    pub(super) fn settings_provider_group(
        &self,
        provider: Provider,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let index = provider as usize;
        let closing = self.closing;
        let configured = self
            .store
            .cli
            .path(provider)
            .map(display_path)
            .unwrap_or_else(|| "自动查找已安装的 CLI".into());
        let has_path = self.store.cli.path(provider).is_some();
        let (effective, available) = match cli::resolve(provider, &self.store.cli) {
            Ok(cli) => (
                format!(
                    "{}：{}{}",
                    cli.source.label(),
                    display_path(&cli.path),
                    if cli.source == cli::PathSource::Environment {
                        "（环境变量优先，覆盖应用路径设置）"
                    } else {
                        ""
                    }
                ),
                true,
            ),
            Err(error) => (format!("尚不可用：{error:#}"), false),
        };
        let probing = self.probes.contains_key(&provider);
        let diagnostic = self
            .diagnostics
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| "尚未检测版本。".into());
        let tested = cli::tested_version(provider);
        let mut cli_footer = format!(
            "路径修改对后续启动生效。检测仅运行 --version；登录请在官方 CLI 完成。已验证版本 {tested}。"
        );
        if provider == Provider::OpenCode {
            cli_footer.push_str(" 模型与服务使用 OpenCode 的原生配置。");
        }

        let choose_view = view.clone();
        let clear_view = view.clone();
        let choose = SettingField::render(move |_, _, _| {
            let choose_view = choose_view.clone();
            let clear_view = clear_view.clone();
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new(("settings-choose", index))
                        .label("选择文件")
                        .small()
                        .outline()
                        .disabled(disabled)
                        .when(!disabled, |button| button.cursor_pointer())
                        .on_click(move |_, window, cx| {
                            choose_view.update(cx, |this, cx| {
                                this.choose_cli(provider, window, cx);
                            });
                        }),
                )
                .when(has_path, |row| {
                    row.child(
                        Button::new(("settings-clear", index))
                            .label("恢复自动查找")
                            .small()
                            .ghost()
                            .disabled(disabled)
                            .when(!disabled, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                clear_view
                                    .update(cx, |this, cx| this.set_cli_path(provider, None, cx));
                            }),
                    )
                })
        });

        let probe_view = view.clone();
        let cancel_view = view.clone();
        let actions = SettingField::render(move |_, _, _| {
            let mut row = h_flex().w_full().justify_end().gap_2();
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

        let guide = SettingField::render(move |_, _, _| {
            Button::new(("engine-guide", index))
                .small()
                .outline()
                .label("安装与登录说明")
                .on_click(move |_, _, cx| {
                    cx.open_url(match provider {
                        Provider::Claude => "https://code.claude.com/docs/en/setup",
                        Provider::Codex => "https://developers.openai.com/codex/cli",
                        Provider::OpenCode => "https://opencode.ai/docs/",
                    })
                })
        });
        let capabilities = vec![
            SettingItem::new(
                if available {
                    "已找到启动文件"
                } else {
                    "需要安装或选择 CLI"
                },
                guide,
            )
            .description("会话沿用 CLI 的登录状态；版本检测不验证认证与模型权限。"),
            SettingItem::new("启动路径", choose)
                .description(configured)
                .disabled(disabled),
            SettingItem::new(
                "实际使用的 CLI",
                if available {
                    paragraph_field(effective)
                } else {
                    error_field(effective)
                },
            )
            .layout(Axis::Vertical),
            SettingItem::new("版本检测", actions).description(diagnostic),
        ];

        settings_section("CLI 路径与版本")
            .items(capabilities)
            .footer(move |_, _| cli_footer.clone())
    }
}
