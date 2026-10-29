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

#[derive(Clone)]
struct ModelRow {
    label: String,
    id: String,
    note: String,
    current: bool,
}

fn model_list_field(provider: Provider, rows: Vec<ModelRow>) -> SettingField<SharedString> {
    let empty = rows.is_empty();
    SettingField::render(move |_, _, _| {
        let mut list = v_flex().w_full().gap_2();
        for row in &rows {
            let show_id = row.label != row.id;
            list = list.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(model_mark(provider, Some(&row.id)))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette().text))
                                    .child(row.label.clone()),
                            )
                            .when(show_id, |this| {
                                this.child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(row.id.clone()),
                                )
                            }),
                    )
                    .when(!row.note.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.))
                                .text_color(rgb(palette().faint))
                                .child(row.note.clone()),
                        )
                    })
                    .when(row.current, |this| {
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
    })
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

    fn providers_group(&self, disabled: bool, view: Entity<Self>) -> SettingGroup {
        let providers = self.store.providers.clone();
        let rows: Vec<_> = providers
            .iter()
            .map(|provider| {
                let in_use = self
                    .store
                    .workspaces
                    .iter()
                    .flat_map(|w| &w.sessions)
                    .any(|s| s.channel == ModelChannel::Provider(provider.id.clone()));
                (
                    provider.clone(),
                    matches!(provider.source, ProviderSource::Ccs { .. }),
                    in_use,
                )
            })
            .collect();
        let empty = rows.is_empty();
        let keywords: Vec<SharedString> = providers
            .iter()
            .map(|provider| provider.name.clone().into())
            .collect();
        let add_view = view.clone();
        let add = SettingItem::render(move |_, _, _| {
            let add_view = add_view.clone();
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.))
                        .line_height(relative(1.5))
                        .text_color(rgb(palette().faint))
                        .child("添加 API 端点、模型列表和凭证环境变量，即可在聊天模型面板中选择。"),
                )
                .child(
                    Button::new("add-custom-provider")
                        .small()
                        .ghost()
                        .label("添加 Provider")
                        .disabled(disabled)
                        .when(!disabled, |button| button.cursor_pointer())
                        .on_click(move |_, window, cx| {
                            add_view
                                .update(cx, |app, cx| app.open_provider_editor(None, window, cx))
                        }),
                )
        })
        .keywords(["添加", "自定义"]);
        let list_view = view;
        let list = SettingItem::render(move |_, _, _| {
            let mut list = v_flex().w_full().gap_3();
            if empty {
                list = list.child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child("尚未添加 Provider。"),
                );
            }
            for (index, (provider, imported, in_use)) in rows.iter().enumerate() {
                let imported = *imported;
                let in_use = *in_use;
                let edit_view = list_view.clone();
                let delete_view = list_view.clone();
                let edit_provider = provider.clone();
                let id = provider.id.clone();
                list = list.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(provider_icon(provider.engine))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette().text))
                                        .child(provider.name.clone()),
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
                                .when(!disabled && !imported, |button| button.cursor_pointer())
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
                                .when(!disabled && !in_use && !imported, |button| {
                                    button.cursor_pointer()
                                })
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
            list
        })
        .keywords(keywords);
        SettingGroup::new()
            .title("Provider 列表")
            .items(vec![add, list])
    }

    fn models_source_group(&self, view: Entity<Self>) -> SettingGroup {
        let closing = self.closing;
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
        let ccs_summary = format!(
            "CCS {} 个 Provider · {} 个模型",
            self.ccs.providers.len(),
            ccs_model_count
        );
        let refresh_view = view;
        let refresh = SettingField::render(move |_, _, _| {
            let refresh_view = refresh_view.clone();
            Button::new("settings-models-refresh")
                .label("同步配置")
                .small()
                .ghost()
                .disabled(closing)
                .when(!closing, |button| button.cursor_pointer())
                .on_click(move |_, window, cx| {
                    refresh_view.update(cx, |this, cx| {
                        this.reload_all_models_async(true, window, cx);
                    });
                })
        });
        let mut items = vec![
            value_item("Claude Code", claude_path),
            value_item("Codex", codex_path),
            value_item("cc-switch", display_path(&self.ccs.database_path)),
            SettingItem::new("同步", refresh)
                .description(ccs_summary)
                .disabled(closing),
        ];
        if let Some(error) = self.ccs.error.clone() {
            items.push(
                SettingItem::new("读取失败", error_field(error))
                    .layout(Axis::Vertical)
                    .keywords(["错误"]),
            );
        } else if let Some(note) = self.ccs.note.clone() {
            items.push(
                SettingItem::new("说明", paragraph_field(note))
                    .layout(Axis::Vertical)
                    .keywords(["说明"]),
            );
        }
        SettingGroup::new()
            .title("模型来源")
            .items(items)
            .footer(|_, _| {
                "CCS 中的 Provider 配置会自动同步到 Provider 列表，与自定义 Provider 一起供聊天选择。同步不会切换 cc-switch 当前 Provider。"
            })
    }

    fn models_provider_group(&self, provider: Provider) -> SettingGroup {
        let catalog = self.catalogs.get(&provider);
        let current = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| "CLI 默认".into());
        let config_path = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let model_count = catalog.map(|catalog| catalog.models.len()).unwrap_or(0);
        let rows: Vec<_> = catalog
            .map(|catalog| {
                let current = catalog.current.as_deref();
                catalog
                    .models
                    .iter()
                    .map(|model| ModelRow {
                        label: model.label.clone(),
                        id: model.id.clone(),
                        note: model.note.clone(),
                        current: current == Some(model.id.as_str()),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let error = catalog.and_then(|catalog| catalog.error.clone());
        let note = catalog.and_then(|catalog| catalog.note.clone());
        let mut items = vec![
            value_item("配置", config_path),
            value_item("当前默认", current),
            value_item("可用模型", format!("{model_count} 个")),
            SettingItem::new("模型列表", model_list_field(provider, rows)).layout(Axis::Vertical),
        ];
        if let Some(error) = error {
            items.push(
                SettingItem::new("读取失败", error_field(error))
                    .layout(Axis::Vertical)
                    .keywords(["错误"]),
            );
        }
        let group = SettingGroup::new().title(provider.label()).items(items);
        if let Some(note) = note {
            group.footer(move |_, _| note.clone())
        } else {
            group
        }
    }

    pub(in crate::app) fn settings_models_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        settings_page("模型同步", FullIconName::Layers).groups(vec![
            self.providers_group(disabled, view.clone()),
            self.models_source_group(view),
            self.models_provider_group(Provider::Claude),
            self.models_provider_group(Provider::Codex),
            self.models_provider_group(Provider::OpenCode),
        ])
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
