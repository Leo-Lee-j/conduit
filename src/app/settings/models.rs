//! Models settings.

use super::*;

struct ProviderEditor {
    app: Entity<ConduitApp>,
    id: Option<String>,
    engine: Provider,
    fields: [Entity<InputState>; 4],
    error: Option<String>,
}

impl ProviderEditor {
    fn new(
        app: Entity<ConduitApp>,
        provider: Option<ModelProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let values = provider
            .as_ref()
            .map(|p| {
                [
                    p.name.clone(),
                    p.base_url.clone(),
                    p.api_key_env.clone(),
                    p.models.join(", "),
                ]
            })
            .unwrap_or_default();
        let placeholders = [
            "例如：我的代理",
            "https://api.example.com/v1",
            "例如：MY_PROVIDER_API_KEY（可留空）",
            "模型 ID，多个用逗号分隔",
        ];
        let fields = std::array::from_fn(|i| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholders[i])
                    .default_value(values[i].clone())
            })
        });
        Self {
            app,
            id: provider.as_ref().map(|p| p.id.clone()),
            engine: provider.map(|p| p.engine).unwrap_or(Provider::Codex),
            fields,
            error: None,
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values: Vec<_> = self
            .fields
            .iter()
            .map(|f| f.read(cx).value().trim().to_string())
            .collect();
        let mut ids = vec![];
        for id in values[3]
            .split([',', '，', ';', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !ids.iter().any(|s| s == id) {
                ids.push(id.to_string());
            }
        }
        let provider = ModelProvider {
            id: self.id.clone().unwrap_or_else(|| {
                format!(
                    "custom-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                )
            }),
            name: values[0].clone(),
            engine: self.engine,
            base_url: values[1].clone(),
            api_key_env: values[2].clone(),
            models: ids,
            source: Default::default(),
            model_metadata: vec![],
        };
        if let Err(error) = provider.validate() {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let saved = self.app.update(cx, |app, cx| {
            if !app.writable || app.closing {
                return false;
            }
            if let Some(current) = app.store.providers.iter_mut().find(|p| p.id == provider.id) {
                *current = provider;
            } else {
                app.store.providers.push(provider);
            }
            let saved = app.persist();
            cx.notify();
            saved
        });
        if saved {
            window.close_dialog(cx);
        } else {
            self.error = Some("保存失败，请检查存储状态".into());
            cx.notify();
        }
    }
}

impl Render for ProviderEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut form = v_flex().id("provider-editor-body").debug_selector(|| "provider-editor-body".into()).w_full().max_h((window.viewport_size().height - px(150.)).max(px(240.))).overflow_y_scroll().gap_3()
            .child(div().text_size(px(11.)).text_color(rgb(palette().faint)).child("按会话选择 Provider，不修改 CLI 的全局配置。Codex 使用 Responses 协议，Claude Code 使用 Anthropic 协议。"));
        let mut engines = h_flex().gap_2();
        for (i, engine) in [Provider::Codex, Provider::Claude].into_iter().enumerate() {
            engines = engines.child(
                Button::new(("custom-provider-engine", i))
                    .ghost()
                    .small()
                    .label(engine.label())
                    .disabled(self.id.is_some())
                    .cursor_pointer()
                    .when(self.engine == engine, |b| {
                        b.bg(rgb(palette().surface_hover))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.engine = engine;
                        cx.notify();
                    })),
            );
        }
        form = form.child(engines);
        for (i, label) in ["名称", "Base URL", "API Key 环境变量名", "模型列表"]
            .into_iter()
            .enumerate()
        {
            form = form.child(
                v_flex()
                    .debug_selector(move || format!("provider-editor-field-{i}"))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(label),
                    )
                    .child(Input::new(&self.fields[i]).h(px(32.)).aria_label(label)),
            );
        }
        form.child(div().text_size(px(10.)).line_height(relative(1.5)).text_color(rgb(palette().faint)).child("填写保存 API Key 的环境变量名称；应用只保存变量名。修改系统环境变量后请重新启动 Conduit。"))
            .when_some(self.error.clone(), |c, error| c.child(div().text_size(px(11.)).text_color(rgb(palette().danger_text)).child(error)))
            .child(h_flex().debug_selector(|| "provider-editor-actions".into()).justify_end().gap_2()
                .child(Button::new("custom-provider-cancel").ghost().small().label("取消").cursor_pointer().on_click(|_, w, cx| w.close_dialog(cx)))
                .child(Button::new("custom-provider-save").primary().small().label("保存").cursor_pointer().on_click(cx.listener(|this, _, w, cx| this.save(w, cx)))))
    }
}

impl ConduitApp {
    fn open_provider_editor(
        &mut self,
        provider: Option<ModelProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let title = if provider.is_some() {
            "编辑 Provider"
        } else {
            "添加 Provider"
        };
        let app = cx.entity();
        let editor = cx.new(|cx| ProviderEditor::new(app, provider, window, cx));
        window.open_dialog(cx, move |dialog, _, _| {
            let editor = editor.clone();
            dialog
                .w(px(480.))
                .title(title)
                .content(move |content, _, _| content.child(editor.clone()))
        });
    }

    fn providers_card(&self, view: Entity<Self>) -> AnyElement {
        let add_view = view.clone();
        let disabled = !self.writable || self.closing;
        let mut card = settings_card(
            "Provider 列表",
            Button::new("add-custom-provider")
                .small()
                .ghost()
                .label("添加 Provider")
                .disabled(disabled)
                .cursor_pointer()
                .on_click(move |_, window, cx| {
                    add_view.update(cx, |app, cx| app.open_provider_editor(None, window, cx))
                })
                .into_any_element(),
        );
        if self.store.providers.is_empty() {
            card = card.child(
                div()
                    .px_4()
                    .py_3()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child("添加 API 端点、模型列表和凭证环境变量，即可在聊天模型面板中选择。"),
            );
        }
        for (index, provider) in self.store.providers.iter().enumerate() {
            let edit_view = view.clone();
            let delete_view = view.clone();
            let provider = provider.clone();
            let edit_provider = provider.clone();
            let id = provider.id.clone();
            let imported = matches!(provider.source, ProviderSource::Ccs { .. });
            let in_use = self
                .store
                .workspaces
                .iter()
                .flat_map(|w| &w.sessions)
                .any(|s| s.channel == ModelChannel::Provider(id.clone()));
            card = card.child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(provider_icon(provider.engine))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette().text))
                                    .child(provider.name),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(format!(
                                        "{} · {} 个模型 · {} · {}",
                                        provider.engine.label(),
                                        provider.models.len(),
                                        provider.base_url,
                                        if imported { "来自 CCS" } else { "自定义" }
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("edit-custom-provider", index))
                            .small()
                            .ghost()
                            .label("编辑")
                            .disabled(disabled || imported)
                            .tooltip(if imported {
                                "此 Provider 的配置跟随 CCS 同步，请在 cc-switch 编辑后同步"
                            } else {
                                "编辑 Provider"
                            })
                            .cursor_pointer()
                            .on_click(move |_, window, cx| {
                                edit_view.update(cx, |app, cx| {
                                    app.open_provider_editor(
                                        Some(edit_provider.clone()),
                                        window,
                                        cx,
                                    )
                                })
                            }),
                    )
                    .child(
                        Button::new(("delete-custom-provider", index))
                            .small()
                            .ghost()
                            .label("删除")
                            .disabled(disabled || in_use || imported)
                            .cursor_pointer()
                            .tooltip(if imported {
                                "此 Provider 的配置由 CCS 同步管理"
                            } else if in_use {
                                "请先将使用此 Provider 的会话切换到其他渠道"
                            } else {
                                "删除 Provider"
                            })
                            .on_click(move |_, _, cx| {
                                delete_view.update(cx, |app, cx| {
                                    if !app.writable
                                        || app.closing
                                        || app
                                            .store
                                            .workspaces
                                            .iter()
                                            .flat_map(|w| &w.sessions)
                                            .any(|s| {
                                                s.channel == ModelChannel::Provider(id.clone())
                                            })
                                    {
                                        return;
                                    }
                                    app.store.providers.retain(|p| p.id != id);
                                    app.persist();
                                    cx.notify();
                                })
                            }),
                    ),
            );
        }
        card.into_any_element()
    }
    pub(in crate::app) fn reload_all_models_async(
        &mut self,
        refresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_opencode_models(refresh, cx);
        // Reading the CLI config files and the cc-switch SQLite database must
        // not block the first frame of the settings dialog (or the UI thread at
        // all); publish the result when the background read finishes.
        cx.spawn_in(window, async move |this, cx| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    (
                        models::load(Provider::Claude),
                        models::load(Provider::Codex),
                        ccs::load(),
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.catalogs.insert(Provider::Claude, loaded.0);
                this.catalogs.insert(Provider::Codex, loaded.1);
                this.ccs = loaded.2;
                this.store.sync_ccs_providers(&this.ccs);
                cx.emit(crate::providers::ProviderCatalogChanged);
                if this.writable {
                    this.dirty = true;
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(in crate::app) fn settings_models_content(&self, view: Entity<Self>) -> AnyElement {
        let refresh_view = view.clone();
        let source_row = |label: &'static str, value: String, divider: bool| -> AnyElement {
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
        let claude_path = self
            .catalogs
            .get(&Provider::Claude)
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let codex_path = self
            .catalogs
            .get(&Provider::Codex)
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let ccs_model_count: usize = self
            .ccs
            .providers
            .iter()
            .map(|entry| entry.models.len())
            .sum();
        let ccs_note = self.ccs.error.clone().or_else(|| self.ccs.note.clone());
        let mut source_card = v_flex()
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
                            .child("模型来源"),
                    )
                    .child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgb(palette().surface_hover))
                            .text_size(px(9.))
                            .text_color(rgb(palette().faint))
                            .child("只读"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!(
                                "CCS {} 个 Provider · {} 个模型",
                                self.ccs.providers.len(),
                                ccs_model_count
                            )),
                    )
                    .child(
                        Button::new("settings-models-refresh")
                            .label("同步配置")
                            .small()
                            .ghost()
                            .disabled(self.closing)
                            .when(!self.closing, |button| button.cursor_pointer())
                            .on_click(move |_, window, cx| {
                                refresh_view.update(cx, |this, cx| {
                                    this.reload_all_models_async(true, window, cx);
                                });
                            }),
                    ),
            )
            .child(source_row("Claude Code", claude_path, false))
            .child(source_row("Codex", codex_path, true))
            .child(source_row(
                "cc-switch",
                display_path(&self.ccs.database_path),
                true,
            ));
        if let Some(note) = ccs_note {
            source_card = source_card.child(
                div()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .text_size(px(10.))
                    .text_color(rgb(if self.ccs.error.is_some() {
                        palette().danger_text
                    } else {
                        palette().faint
                    }))
                    .child(note),
            );
        }
        let mut page = v_flex()
            .w_full()
            .gap_4()
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("CCS 中的 Provider 配置会自动同步到下方 Provider 列表，与自定义 Provider 一起供聊天选择。同步不会切换 cc-switch 当前 Provider。"),
            )
            .child(self.providers_card(view.clone()))
            .child(source_card);
        for provider in Provider::ALL {
            page = page.child(self.settings_models_provider(provider));
        }
        page.into_any_element()
    }

    pub(in crate::app) fn settings_models_provider(&self, provider: Provider) -> AnyElement {
        let catalog = self.catalogs.get(&provider);
        let current = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| "CLI 默认".into());
        let config_path = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let model_count = catalog.map(|catalog| catalog.models.len()).unwrap_or(0);
        let badge = |text: &'static str| -> AnyElement {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgb(palette().surface_hover))
                .text_size(px(9.))
                .text_color(rgb(palette().muted))
                .child(text)
                .into_any_element()
        };
        let model_row =
            |label: String, id: String, note: String, tag: Option<&'static str>| -> AnyElement {
                let mark = model_mark(provider, Some(&id));
                let mut value = v_flex().flex_1().min_w_0().gap(px(1.)).child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(palette().text))
                        .child(label.clone()),
                );
                if label != id {
                    value = value.child(
                        div()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(id),
                    );
                }
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(mark)
                    .child(value)
                    .when(!note.is_empty(), |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(note),
                        )
                    })
                    .when_some(tag, |row, tag| row.child(badge(tag)))
                    .into_any_element()
            };
        let mut current_card = v_flex()
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
                            .text_size(px(12.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child("当前 CLI 模型"),
                    )
                    .child(badge("CLI"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!("{model_count} 个")),
                    ),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_4()
                    .items_center()
                    .child(settings_label("配置"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(config_path),
                    ),
            );
        if let Some(catalog) = catalog {
            for model in &catalog.models {
                current_card = current_card.child(model_row(
                    model.label.clone(),
                    model.id.clone(),
                    model.note.clone(),
                    (catalog.current.as_deref() == Some(model.id.as_str())).then_some("CLI 当前"),
                ));
            }
            if catalog.models.is_empty() {
                current_card = current_card.child(
                    div()
                        .px_4()
                        .py_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("未发现模型定义，可在 CLI 中配置后重新读取。"),
                );
            }
            if let Some(error) = &catalog.error {
                current_card = current_card.child(
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
                current_card = current_card.child(
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
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(provider.label()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!("当前默认 {current}")),
                    ),
            )
            .child(current_card)
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, ProviderEditor, Store};
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn provider_editor_renders_all_fields_and_save_actions(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
            let editor = cx.new(|cx| ProviderEditor::new(app, None, window, cx));
            gpui_kit::component::Root::new(editor, window, cx)
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let mut previous = cx
            .debug_bounds("provider-editor-field-0")
            .expect("name field");
        for selector in [
            "provider-editor-field-1",
            "provider-editor-field-2",
            "provider-editor-field-3",
        ] {
            let field = cx.debug_bounds(selector).expect("provider field");
            assert!(field.top() >= previous.bottom());
            previous = field;
        }
        let actions = cx
            .debug_bounds("provider-editor-actions")
            .expect("save and cancel actions");
        assert!(actions.top() > previous.bottom());
    }
}
