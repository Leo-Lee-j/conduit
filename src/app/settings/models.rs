//! Models settings.

use super::*;

impl ConduitApp {
    pub(in crate::app) fn reload_all_models_async(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                            .label("重新读取")
                            .small()
                            .ghost()
                            .disabled(self.closing)
                            .when(!self.closing, |button| button.cursor_pointer())
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |this, cx| {
                                    this.reload_all_models(cx);
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
                    .child("当前模型来自 CLI 正在使用的配置；CCS 模型来自 cc-switch 的 Provider 目录。只读取模型名与上下文信息，不查询凭证，也不修改任一配置。"),
            )
            .child(source_card);
        for provider in [Provider::Claude, Provider::Codex] {
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
        let providers: Vec<_> = self.ccs.for_provider(provider).collect();
        let ccs_count: usize = providers.iter().map(|entry| entry.models.len()).sum();
        let mut ccs_card = v_flex()
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
                            .child("CCS 同步"),
                    )
                    .child(badge("只读"))
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!(
                                "{} 个 Provider · {ccs_count} 个模型",
                                providers.len()
                            )),
                    ),
            );
        for entry in &providers {
            ccs_card = ccs_card.child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(entry.name.clone()),
                    )
                    .when(entry.current, |row| row.child(badge("当前 Provider")))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .child(format!("{} 个模型", entry.models.len())),
                    ),
            );
            for model in &entry.models {
                ccs_card = ccs_card.child(model_row(
                    model.label.clone(),
                    model.id.clone(),
                    model.note.clone(),
                    None,
                ));
            }
        }
        if providers.is_empty() {
            ccs_card = ccs_card.child(
                div()
                    .px_4()
                    .py_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child(if provider == Provider::Claude {
                        "cc-switch 中没有可列出的 Claude 模型目录。"
                    } else {
                        "cc-switch 中没有可列出的 Codex 模型目录。"
                    }),
            );
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
            .child(ccs_card)
            .into_any_element()
    }
}
