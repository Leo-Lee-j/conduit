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

    pub(in crate::app) fn settings_provider_content(
        &self,
        provider: Provider,
        view: Entity<Self>,
    ) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let label = |text: &'static str| {
            div()
                .w(px(84.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(palette().faint))
                .child(text)
        };
        let index: usize = match provider {
            Provider::Claude => 0,
            Provider::Codex => 1,
        };
        let configured = self
            .store
            .cli
            .path(provider)
            .map(display_path)
            .unwrap_or_else(|| "自动查找（环境变量 / PATH）".into());
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
        let choose_view = view.clone();
        let clear_view = view.clone();
        let mut actions = h_flex()
            .px_4()
            .py_2()
            .gap_2()
            .justify_end()
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(
                Button::new(("settings-clear", index))
                    .label("清除配置")
                    .small()
                    .ghost()
                    .disabled(disabled || self.store.cli.path(provider).is_none())
                    .when(
                        !disabled && self.store.cli.path(provider).is_some(),
                        |button| button.cursor_pointer(),
                    )
                    .on_click(move |_, _, cx| {
                        clear_view.update(cx, |this, cx| {
                            this.set_cli_path(provider, None, cx);
                        });
                    }),
            );
        if probing {
            let cancel_view = view.clone();
            actions = actions.child(
                Button::new(("settings-cancel-probe", index))
                    .label("取消检测")
                    .small()
                    .ghost()
                    .disabled(self.closing)
                    .when(!self.closing, |button| button.cursor_pointer())
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
            let probe_view = view.clone();
            actions = actions.child(
                Button::new(("settings-probe", index))
                    .label("检测版本")
                    .small()
                    .outline()
                    .disabled(self.closing)
                    .when(!self.closing, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        probe_view.update(cx, |this, cx| {
                            this.probe_cli(provider, cx);
                        });
                    }),
            );
        }
        let card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("可执行文件"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!("已验证版本 {}", cli::tested_version(provider))),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .child(label("可执行文件"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(configured),
                    )
                    .child(
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
                            }),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(label("本次启动"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(effective),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_start()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(label("状态"))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
                            .child(diagnostic),
                    ),
            )
            .child(actions);
        let catalog = self.catalogs.get(&provider);
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
        let mut model_rows = v_flex().w_full();
        if let Some(catalog) = catalog {
            for model in &catalog.models {
                let is_current = catalog.current.as_deref() == Some(model.id.as_str());
                let show_id = model.label != model.id;
                let mut value = v_flex().flex_1().min_w_0().gap(px(1.)).child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(palette().text))
                        .child(model.label.clone()),
                );
                if show_id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(model.id.clone()),
                    );
                }
                model_rows = model_rows.child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .gap_3()
                        .items_center()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(model_mark(provider, Some(&model.id)))
                        .child(value)
                        .when(!model.note.is_empty(), |row| {
                            row.child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(model.note.clone()),
                            )
                        })
                        .when(is_current, |row| {
                            row.child(
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
            if catalog.models.is_empty() {
                model_rows = model_rows.child(
                    h_flex()
                        .px_4()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
        }
        let sync_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
            let mut row = h_flex().px_4().py_3().gap_4().items_center();
            if divider {
                row = row.border_t_1().border_color(rgb(palette().border));
            }
            row.child(settings_label(label))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(value),
                )
                .into_any_element()
        };
        let sync_view = view.clone();
        let mut model_card = v_flex()
            .w_full()
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("当前模型"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_hover))
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
                            .child("CLI"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(("settings-sync-models", index))
                            .label("重新读取")
                            .small()
                            .ghost()
                            .disabled(self.closing)
                            .when(!self.closing, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                sync_view.update(cx, |this, cx| {
                                    this.reload_models(provider, cx);
                                });
                            }),
                    ),
            )
            .child(sync_row("来源", source, false));
        if let Some(catalog_path) = catalog_path {
            model_card = model_card.child(sync_row("模型目录", catalog_path, true));
        }
        model_card = model_card
            .child(sync_row("当前默认", current_model, true))
            .child(sync_row("接口", endpoint, true));
        if let Some(detail) = detail {
            model_card = model_card.child(sync_row("附加配置", detail, true));
        }
        model_card = model_card
            .child(sync_row("可用模型", format!("{model_count} 个"), true))
            .child(model_rows);
        if let Some(catalog) = catalog {
            if let Some(error) = &catalog.error {
                model_card = model_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().danger_text))
                        .child(error.clone()),
                );
            } else if let Some(note) = &catalog.note {
                model_card = model_card.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(10.))
                        .text_color(rgb(palette().faint))
                        .child(note.clone()),
                );
            }
        }
        v_flex()
            .w_full()
            .gap_3()
            .child(card)
            .child(model_card)
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("版本检测只运行 --version，不调用模型，也不验证登录状态；请先在官方 CLI 中完成登录。"),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("此处只显示 CLI 当前模型；完整来源与 CCS 同步见左侧「模型同步」。不读取或显示密钥，也不会写回 CLI 配置。"),
            )
            .into_any_element()
    }

    pub(in crate::app) fn settings_cli_content(&self) -> AnyElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("路径查找顺序"),
                            )
                            .child(div().flex_1())
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child("前者优先"),
                            ),
                    )
                    .child(settings_row(
                        "环境变量",
                        "CONDUIT_CLAUDE_PATH · CONDUIT_CODEX_PATH".into(),
                        false,
                    ))
                    .child(settings_row(
                        "应用配置",
                        "在本页 Claude Code / Codex 中手动指定".into(),
                        true,
                    ))
                    .child(settings_row(
                        "PATH",
                        "claude.exe / .cmd · codex.exe / .cmd".into(),
                        true,
                    ))
                    .child(settings_row(
                        "常见安装位置",
                        "~/.local/bin · %LOCALAPPDATA%\\Programs\\OpenAI\\Codex".into(),
                        true,
                    )),
            )
            .child(
                v_flex()
                    .w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .child(
                        h_flex()
                            .px_4()
                            .py_3()
                            .items_center()
                            .border_b_1()
                            .border_color(rgb(palette().border))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child("可执行文件与版本"),
                            ),
                    )
                    .child(settings_row(
                        "Claude Code",
                        "在左侧「Claude Code」页面选择本机 .exe / .cmd 并检测版本".into(),
                        false,
                    ))
                    .child(settings_row(
                        "Codex",
                        "在左侧「Codex」页面选择本机 .exe / .cmd 并检测版本".into(),
                        true,
                    )),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("修改 CLI 路径或版本设置仅影响后续启动的会话；路径无效时会显式报错，不会自动改用另一份程序。"),
            )
            .into_any_element()
    }
}
