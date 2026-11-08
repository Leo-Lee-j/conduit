//! Models settings.

use super::*;

struct ProviderEditor {
    app: Entity<ConduitApp>,
    id: Option<String>,
    engine: Provider,
    fields: [Entity<InputState>; 4],
    error: Option<String>,
    key: Entity<InputState>,
    protected_key: Option<String>,
    env_auth: bool,
    probing: bool,
    feedback: Option<String>,
    subscriptions: Vec<Subscription>,
    initial: ([String; 4], Provider, bool, Option<String>),
    confirm_discard: bool,
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
            conduit::i18n::text_static("settings.models.e_g_my_proxy"),
            "https://api.example.com/v1",
            conduit::i18n::text_static("settings.models.e_g_my_provider_api_key_optional"),
            conduit::i18n::text_static("settings.models.model_ids_separated_by_commas"),
        ];
        let fields = std::array::from_fn(|i| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholders[i])
                    .default_value(values[i].clone())
            })
        });
        let engine = provider.as_ref().map(|p| p.engine).unwrap_or_else(|| {
            let engine = app.read(cx).connection_settings.engine;
            if engine == Provider::OpenCode {
                Provider::Codex
            } else {
                engine
            }
        });
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(conduit::i18n::text_static(
                    "settings.models.enter_api_key_leave_empty_to_keep",
                ))
        });
        let subscriptions = fields[..3]
            .iter()
            .chain(std::iter::once(&key))
            .map(|field| {
                cx.subscribe(field, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.feedback = None;
                        this.error = None;
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            initial: (
                values,
                engine,
                provider.as_ref().is_some_and(|p| !p.api_key_env.is_empty()),
                provider.as_ref().and_then(|p| p.protected_api_key.clone()),
            ),
            confirm_discard: false,
            key,
            protected_key: provider.as_ref().and_then(|p| p.protected_api_key.clone()),
            env_auth: provider.as_ref().is_some_and(|p| !p.api_key_env.is_empty()),
            probing: false,
            feedback: None,
            subscriptions,
            app,
            id: provider.as_ref().map(|p| p.id.clone()),
            engine,
            fields,
            error: None,
        }
    }

    fn can_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.probing {
            return false;
        }
        let dirty = self
            .fields
            .iter()
            .zip(&self.initial.0)
            .any(|(field, value)| field.read(cx).value().as_str() != value)
            || self.engine != self.initial.1
            || self.env_auth != self.initial.2
            || self.protected_key != self.initial.3
            || !self.key.read(cx).value().is_empty();
        if dirty {
            self.confirm_discard = true;
            cx.notify();
        }
        !dirty
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.probing {
            return;
        }
        let protected_key = if self.env_auth {
            None
        } else if self.key.read(cx).value().trim().is_empty() {
            self.protected_key.clone()
        } else {
            match crate::providers::credentials::protect(self.key.read(cx).value().trim()) {
                Ok(key) => Some(key),
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        };
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
            api_key_env: if self.env_auth {
                values[2].clone()
            } else {
                String::new()
            },
            protected_api_key: protected_key,
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
            cx.emit(crate::providers::ProviderCatalogChanged);
            let saved = app.persist();
            cx.notify();
            saved
        });
        if saved {
            window.close_dialog(cx);
        } else {
            self.error = Some(
                conduit::i18n::text_static("settings.models.save_failed_check_storage_status")
                    .into(),
            );
            cx.notify();
        }
    }
}

impl ProviderEditor {
    fn discover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.probing {
            return;
        }
        let key = if self.env_auth {
            let name = self.fields[2].read(cx).value().trim().to_owned();
            if name.is_empty() {
                Ok(None)
            } else {
                std::env::var(&name).map(Some).map_err(|_| {
                    anyhow::anyhow!(conduit::i18n::format(
                        "settings.models.environment_variable_not_found_you_can_also",
                        &[("name", name.to_string())]
                    ))
                })
            }
        } else if !self.key.read(cx).value().trim().is_empty() {
            Ok(Some(self.key.read(cx).value().trim().to_owned()))
        } else {
            self.protected_key
                .as_deref()
                .map(crate::providers::credentials::unprotect)
                .transpose()
        };
        let key = match key {
            Ok(key) => key,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let base = self.fields[1].read(cx).value().trim().to_owned();
        if let Err(error) = crate::providers::discovery::endpoint(&base) {
            self.error = Some(error.to_string());
            cx.notify();
            return;
        }
        let engine = self.engine;
        self.probing = true;
        self.error = None;
        self.feedback = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        crate::providers::discovery::fetch(engine, &base, key.as_deref())
                    })
                    .await;
            let _ = this.update_in(cx, |editor, window, cx| {
                editor.probing = false;
                match result {
                    Ok(models) => {
                        let mut merged = editor.fields[3]
                            .read(cx)
                            .value()
                            .split([',', '，', ';', '\n'])
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        for model in &models {
                            if !merged.contains(model) {
                                merged.push(model.clone());
                            }
                        }
                        editor.fields[3].update(cx, |field, cx| {
                            field.set_value(merged.join(", "), window, cx)
                        });
                        editor.feedback = Some(conduit::i18n::format(
                            "settings.models.catalog_request_succeeded_read_models_generation_has",
                            &[("arg0", format!("{}", models.len()))],
                        ));
                    }
                    Err(error) => editor.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for ProviderEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.subscriptions;
        let mut form = v_flex()
            .id("provider-editor-body")
            .debug_selector(|| "provider-editor-body".into())
            .w_full()
            .max_h((window.viewport_size().height - px(190.)).max(px(180.)))
            .overflow_y_scroll()
            .gap_3()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "settings.models.configure_the_endpoint_and_credentials_then_read",
                    )),
            );
        let mut engines = h_flex().gap_2();
        for (i, engine) in [Provider::Codex, Provider::Claude].into_iter().enumerate() {
            engines = engines.child(
                Button::new(("custom-provider-engine", i))
                    .outline()
                    .small()
                    .label(engine.label())
                    .disabled(self.id.is_some() || self.probing)
                    .selected(self.engine == engine)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.engine = engine;
                        this.feedback = None;
                        cx.notify();
                    })),
            );
        }
        form = form.child(engines);
        for (i, label) in [
            conduit::i18n::text_static("settings.config_editor.service_name"),
            "Base URL",
        ]
        .into_iter()
        .enumerate()
        {
            form = form.child(
                v_flex()
                    .debug_selector(move || format!("provider-editor-field-{i}"))
                    .gap_1()
                    .child(label)
                    .child(
                        Input::new(&self.fields[i])
                            .disabled(self.probing)
                            .aria_label(label),
                    ),
            );
        }
        let env_auth = self.env_auth;
        form = form.child(
            v_flex()
                .debug_selector(|| "provider-editor-field-2".into())
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("auth-direct")
                                .small()
                                .ghost()
                                .label("API Key")
                                .selected(!env_auth)
                                .disabled(self.probing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.env_auth = false;
                                    this.feedback = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("auth-env")
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static(
                                    "settings.models.environment_variable",
                                ))
                                .selected(env_auth)
                                .disabled(self.probing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.env_auth = true;
                                    this.feedback = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(if env_auth {
                    Input::new(&self.fields[2])
                        .disabled(self.probing)
                        .aria_label(conduit::i18n::text_static(
                            "settings.models.api_key_environment_variable_name",
                        ))
                } else {
                    Input::new(&self.key)
                        .mask_toggle()
                        .disabled(self.probing)
                        .aria_label("API Key")
                })
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(if env_auth {
                            conduit::i18n::text_static(
                                "settings.models.only_the_variable_name_is_saved_restart",
                            )
                        } else if self.protected_key.is_some() {
                            conduit::i18n::text_static(
                                "settings.models.credentials_saved_leave_empty_to_keep_them",
                            )
                        } else {
                            conduit::i18n::text_static(
                                "settings.models.credentials_are_encrypted_for_the_current_windows",
                            )
                        }),
                )
                .when(!env_auth && self.protected_key.is_some(), |body| {
                    body.child(
                        Button::new("clear-provider-key")
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static(
                                "settings.models.remove_saved_credentials",
                            ))
                            .disabled(self.probing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.protected_key = None;
                                this.key.update(cx, |key, cx| key.set_value("", window, cx));
                                cx.notify();
                            })),
                    )
                }),
        );
        form = form.child(
            v_flex()
                .debug_selector(|| "provider-editor-field-3".into())
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(conduit::i18n::text_static(
                            "settings.models.model_ids_comma_separated",
                        ))
                        .child(
                            Button::new("provider-discover")
                                .small()
                                .outline()
                                .label(if self.probing {
                                    conduit::i18n::text_static("settings.models.requesting")
                                } else {
                                    conduit::i18n::text_static("settings.models.read_model_catalog")
                                })
                                .disabled(self.probing)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.discover(window, cx)),
                                ),
                        ),
                )
                .child(
                    Input::new(&self.fields[3])
                        .disabled(self.probing)
                        .aria_label(conduit::i18n::text_static("settings.models.model_id")),
                ),
        );
        form = form
            .when_some(self.feedback.clone(), |body, message| {
                body.child(div().text_size(ui_text_size(12.)).child(message))
            })
            .when_some(self.error.clone(), |body, error| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().danger_text))
                        .child(error),
                )
            });
        v_flex().w_full().gap_3().child(form).child(
            h_flex()
                .debug_selector(|| "provider-editor-actions".into())
                .justify_end()
                .gap_2()
                .child(
                    Button::new("custom-provider-cancel")
                        .ghost()
                        .small()
                        .label(if self.confirm_discard {
                            conduit::i18n::text_static("settings.config_editor.continue_editing")
                        } else {
                            conduit::i18n::text_static("git_branches.cancel")
                        })
                        .disabled(self.probing)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.confirm_discard {
                                this.confirm_discard = false;
                                this.fields[0].read(cx).focus_handle(cx).focus(window, cx);
                                cx.notify();
                            } else if this.can_close(cx) {
                                window.close_dialog(cx);
                            }
                        })),
                )
                .when(self.confirm_discard, |row| {
                    row.child(
                        Button::new("custom-provider-discard")
                            .label(conduit::i18n::text_static(
                                "settings.config_editor.discard_unsaved_changes",
                            ))
                            .small()
                            .ghost()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                })
                .child(
                    Button::new("custom-provider-save")
                        .primary()
                        .small()
                        .label(conduit::i18n::text_static("settings.models.save_service"))
                        .disabled(self.probing)
                        .on_click(cx.listener(|this, _, w, cx| this.save(w, cx))),
                ),
        )
    }
}

impl ConduitApp {
    pub(super) fn open_provider_editor(
        &mut self,
        provider: Option<ModelProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let title = if provider.is_some() {
            conduit::i18n::text_static("settings.models.edit_model_service")
        } else {
            conduit::i18n::text_static("settings.models.add_model_service")
        };
        let app = cx.entity();
        let editor = cx.new(|cx| ProviderEditor::new(app, provider, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let editor = editor.clone();
            let dismiss = editor.clone();
            let probing = editor.read(cx).probing;
            dialog
                .w(px(
                    (f32::from(window.viewport_size().width) - 60.).clamp(0., 620.)
                ))
                .margin_top(px(32.))
                .overlay_closable(false)
                .keyboard(!probing)
                .close_button(!probing)
                .on_cancel(move |_, _, cx| dismiss.update(cx, |editor, cx| editor.can_close(cx)))
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
        if self.closing || self.connection_settings.syncing {
            return;
        }
        self.connection_settings.syncing = true;
        self.ensure_opencode_models(refresh, cx);
        cx.notify();
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
                this.connection_settings.syncing = false;
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

    pub(super) fn providers_group(
        &self,
        engine: Provider,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let providers: Vec<_> = self
            .store
            .providers
            .iter()
            .filter(|p| p.engine == engine)
            .cloned()
            .collect();
        let rows: Vec<_> = providers
            .iter()
            .map(|provider| {
                let references = self.provider_references(&provider.id);
                let in_use = !references.is_empty();
                (
                    provider.clone(),
                    matches!(provider.source, ProviderSource::Ccs { .. }),
                    in_use,
                    references,
                )
            })
            .collect();
        let empty = rows.is_empty();
        let service_count = rows.len();
        let keywords: Vec<SharedString> = providers
            .iter()
            .map(|provider| provider.name.clone().into())
            .collect();
        let add = SettingItem::render(move |_, _, _| {
            h_flex()
                .w_full()
                .justify_between()
                .child(div().text_size(ui_text_size(14.)).font_semibold().child(
                    conduit::i18n::text_static("settings.defaults.model_services"),
                ))
                .child(management_badge(
                    conduit::i18n::format(
                        "settings.models.services",
                        &[("service_count", format!("{}", service_count))],
                    ),
                    false,
                ))
        })
        .keywords([
            conduit::i18n::text_static("settings.defaults.model_services"),
            "Provider",
            conduit::i18n::text_static("settings.models.add"),
            conduit::i18n::text_static("settings.models.custom"),
        ]);
        let default_service = self
            .store
            .session_defaults
            .get(&engine)
            .and_then(|defaults| match &defaults.channel {
                ModelChannel::Provider(id) => Some(id.clone()),
                _ => None,
            });
        let list_view = view;
        let list = SettingItem::render(move |_, _, _| {
            let mut list = v_flex().w_full().gap_3();
            if empty {
                list = list.child(
                    v_flex().w_full().items_center().p_6().gap_2().rounded_lg().border_1().border_color(rgb(palette().border))
                        .child(Icon::new(FullIconName::Boxes).text_color(rgb(palette().muted)))
                        .text_size(ui_text_size(12.))
                        .line_height(relative(1.6))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("settings.models.no_model_services_yet_add_endpoints_and")),
                );
            }
            for (index, (provider, imported, in_use, references)) in rows.iter().enumerate() {
                let imported = *imported;
                let in_use = *in_use;
                let is_default = default_service.as_ref() == Some(&provider.id);
                let edit_view = list_view.clone();
                let delete_view = list_view.clone();
                let edit_provider = provider.clone();
                let id = provider.id.clone();
                let use_view = list_view.clone();
                let use_provider = provider.clone();
                let references = references.clone();
                let all_references = references.clone();
                list = list.child(
                    h_flex()
                        .w_full()
                        .flex_wrap()
                        .px_4()
                        .py_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(palette().border))
                        .bg(rgb(palette().surface))
                        .when(is_default, |card| card.border_color(rgb(palette().activity_running)).bg(gpui_kit::rgba((palette().activity_running << 8) | 0x0d)))
                        .gap_3()
                        .items_start()
                        .child(h_flex().flex_1().min_w_0().flex_basis(px(220.)).gap_2().items_start().child(provider_icon(provider.engine))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .debug_selector(move || format!("service-description-{index}"))
                                .child(
                                    h_flex().gap_2().flex_wrap()
                                        .child(div().text_size(ui_text_size(13.)).font_semibold().text_color(rgb(palette().text)).child(provider.name.clone()))
                                        .child(management_badge(if imported { "CCS" } else { conduit::i18n::text_static("settings.models.custom") }, false))
                                        .when(is_default, |row| row.child(management_badge(conduit::i18n::text_static("settings.defaults.new_session_defaults"), true)))
                                        .when(in_use, |row| row.child(div().text_size(ui_text_size(11.)).text_color(rgb(palette().muted)).child(conduit::i18n::format("settings.models.references", &[("arg0", format!("{}", references.len()))])))),
                                )
                                .child(
                                    div()
                                        .id(("settings-provider-endpoint", index))
                                        .truncate()
                                        .tooltip({
                                            let endpoint = provider.base_url.clone();
                                            move |window, cx| gpui_kit::component::tooltip::Tooltip::new(endpoint.clone()).build(window, cx)
                                        })
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(provider.base_url.clone()),
                                )
                                .child(div().text_size(ui_text_size(11.)).text_color(rgb(palette().muted)).child(conduit::i18n::format("settings.models.models", &[("arg0", format!("{}", provider.models.len())), ("arg1", (if imported { conduit::i18n::text_static("settings.models.edit_in_cc_switch") } else { "" }).to_string())])))
                                .when(in_use, |body| body.child(div().text_size(ui_text_size(11.)).text_color(rgb(palette().muted)).child(conduit::i18n::format("settings.models.references_6f2d6f", &[("arg0", references.iter().take(3).cloned().collect::<Vec<_>>().join("、").to_string())])))),
                        )
                        )
                        .child(h_flex().flex_shrink_0().justify_end().gap_2().flex_wrap().debug_selector(move || format!("service-actions-{index}"))
                        .child(Button::new(("provider-use-default", index)).small().outline().label(conduit::i18n::text_static("settings.models.use_for_new_sessions"))
                            .disabled(disabled).tooltip(conduit::i18n::text_static("settings.models.set_the_service_and_model_for_new"))
                            .dropdown_menu(move |mut menu, _, _| {
                                for model in &use_provider.models {
                                    let model = model.clone();
                                    let id = use_provider.id.clone();
                                    let engine = use_provider.engine;
                                    let view = use_view.clone();
                                    menu = menu.item(PopupMenuItem::new(model.clone()).on_click(move |_, _, cx| view.update(cx, |app, cx| {
                                        if !app.writable || app.closing { return; }
                                        app.store.session_defaults.insert(engine, crate::store::SessionDefaults { channel: ModelChannel::Provider(id.clone()), model: Some(model.clone()) });
                                        app.persist(); cx.notify();
                                    })));
                                }
                                menu
                            }))
                        .when(in_use, |row| row.child(Button::new(("provider-references", index)).small().ghost().label(conduit::i18n::text_static("settings.models.references_23c0e1"))
                            .on_click(move |_, window, cx| {
                                let references = all_references.clone();
                                window.open_dialog(cx, move |dialog, _, _| {
                                    let references = references.clone();
                                    dialog.title(conduit::i18n::text_static("settings.models.service_references")).content(move |body, _, _| body.child(
                                        div().id("provider-references-list").max_h(px(320.)).overflow_y_scroll()
                                            .child(v_flex().gap_2().children(references.iter().map(|r| div().text_size(ui_text_size(12.)).child(r.clone()))))))
                                });
                            })))
                        .when(!imported, |row| row.child(
                            Button::new(("edit-custom-provider", index))
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("settings.extensions.edit"))
                                .disabled(disabled || imported)
                                .tooltip(if imported {
                                    conduit::i18n::text_static("settings.models.this_provider_follows_ccs_sync_edit_it")
                                } else {
                                    conduit::i18n::text_static("settings.models.edit_provider")
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
                        ))
                        .when(!imported, |row| row.child(
                            Button::new(("delete-custom-provider", index))
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("git_state.deleted"))
                                .disabled(disabled || in_use || imported)
                                .when(!disabled && !in_use && !imported, |button| {
                                    button.cursor_pointer()
                                })
                                .tooltip(if imported {
                                    conduit::i18n::text_static("settings.models.this_provider_s_configuration_is_managed_by")
                                } else if in_use {
                                    conduit::i18n::text_static("settings.models.first_change_the_sessions_naming_settings_or")
                                } else {
                                    conduit::i18n::text_static("settings.models.delete_provider")
                                })
                                .on_click(move |_, _, cx| {
                                    delete_view.update(cx, |app, cx| {
                                        if !app.writable
                                            || app.closing
                                            || !app.provider_references(&id).is_empty()

                                        {
                                            return;
                                        }
                                        app.store.providers.retain(|p| p.id != id);
                                        app.persist();
                                        cx.emit(crate::providers::ProviderCatalogChanged);
                                        cx.notify();
                                    })
                                }),
                        ))),
                );
            }
            list
        })
        .keywords(keywords);
        SettingGroup::new()
            .variant(GroupBoxVariant::Normal)
            .items(vec![add, list])
    }

    pub(super) fn models_source_group(&self, engine: Provider, view: Entity<Self>) -> SettingGroup {
        let expanded = self.connection_settings.sources_expanded;
        let config_path = self
            .catalogs
            .get(&engine)
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let ccs_model_count: usize = self
            .ccs
            .providers
            .iter()
            .map(|entry| entry.models.len())
            .sum();
        let ccs_summary = conduit::i18n::format(
            "settings.models.ccs_providers_models",
            &[
                ("arg0", format!("{}", self.ccs.providers.len())),
                ("arg1", format!("{}", ccs_model_count)),
            ],
        );
        let toggle = SettingItem::render(move |_, _, _| {
            let toggle_view = view.clone();
            Button::new("settings-models-sources")
                .label(if expanded {
                    conduit::i18n::text_static("settings.models.hide_configuration_sources")
                } else {
                    conduit::i18n::text_static("settings.models.view_configuration_sources")
                })
                .small()
                .ghost()
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    toggle_view.update(cx, |this, cx| {
                        this.connection_settings.sources_expanded = !expanded;
                        cx.notify();
                    });
                })
        })
        .keywords([
            conduit::i18n::text_static("settings.models.configuration_sources"),
            "CCS",
            "cc-switch",
        ]);
        let mut items = vec![toggle];
        if expanded {
            items.extend(vec![
                value_item(engine.label(), config_path),
                value_item("cc-switch", display_path(&self.ccs.database_path)),
                value_item(
                    conduit::i18n::text_static("settings.models.loaded"),
                    ccs_summary,
                ),
            ]);
        }
        if let Some(error) = self.ccs.error.clone() {
            items.push(
                SettingItem::new(
                    conduit::i18n::text_static("settings.models.read_failed"),
                    error_field(error),
                )
                .layout(Axis::Vertical)
                .keywords([conduit::i18n::text_static("settings.models.error")]),
            );
        } else if let Some(note) = self.ccs.note.clone().filter(|_| expanded) {
            items.push(
                SettingItem::new(
                    conduit::i18n::text_static("settings.models.description"),
                    paragraph_field(note),
                )
                .layout(Axis::Vertical)
                .keywords([conduit::i18n::text_static("settings.models.description")]),
            );
        }
        SettingGroup::new()
            .variant(GroupBoxVariant::Normal)
            .items(items)
            .footer(|_, _| {
                conduit::i18n::text_static("settings.models.sync_reads_from_clis_and_cc_switch")
            })
    }

    pub(super) fn models_provider_group(
        &self,
        provider: Provider,
        view: Entity<Self>,
    ) -> SettingGroup {
        let catalog = self.engine_catalog(provider);
        let current = catalog
            .and_then(|catalog| catalog.current.clone())
            .unwrap_or_else(|| conduit::i18n::text_static("generation.cli_default").into());
        let config_path = catalog
            .map(|catalog| display_path(&catalog.config_path))
            .unwrap_or_else(|| "—".into());
        let model_count = self.managed_native_models(provider, true).len();
        let error = catalog.and_then(|catalog| catalog.error.clone());
        let note = catalog.and_then(|catalog| catalog.note.clone());
        let expanded = self.connection_settings.models_expanded;
        let closing = self.closing;
        let summary_view = view.clone();
        let summary = SettingItem::render(move |_, _, _| {
            let toggle_view = summary_view.clone();
            let refresh_view = summary_view.clone();
            h_flex()
                .w_full()
                .justify_between()
                .gap_2()
                .flex_wrap()
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::format(
                            "settings.models.model_definitions_generation_not_verified",
                            &[("model_count", format!("{}", model_count))],
                        )),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .when(provider == Provider::OpenCode, |row| {
                            row.child(
                                Button::new("settings-opencode-refresh")
                                    .ghost()
                                    .small()
                                    .label(conduit::i18n::text_static(
                                        "settings.models.refresh_models",
                                    ))
                                    .disabled(closing)
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        refresh_view.update(cx, |app, cx| {
                                            app.ensure_opencode_models(true, cx)
                                        })
                                    }),
                            )
                        })
                        .child(
                            Button::new("settings-models-expand")
                                .ghost()
                                .small()
                                .label(if expanded {
                                    conduit::i18n::text_static("settings.models.hide_model_catalog")
                                } else {
                                    conduit::i18n::text_static(
                                        "settings.models.manage_model_catalog",
                                    )
                                })
                                .cursor_pointer()
                                .on_click(move |_, _, cx| {
                                    toggle_view.update(cx, |app, cx| {
                                        app.connection_settings.models_expanded = !expanded;
                                        cx.notify();
                                    })
                                }),
                        ),
                )
        })
        .keywords([
            conduit::i18n::text_static("settings.models.model_list"),
            conduit::i18n::text_static("chat.default"),
            conduit::i18n::text_static("settings.models.catalog"),
        ]);
        let mut items = vec![
            value_item(
                conduit::i18n::text_static("settings.models.cli_configured_default_model"),
                current,
            ),
            summary,
        ];
        if expanded {
            items.push(value_item(
                conduit::i18n::text_static("settings.general.configuration_file"),
                config_path,
            ));
            if let Some(path) = catalog.and_then(|catalog| catalog.catalog_path.as_deref()) {
                items.push(value_item(
                    conduit::i18n::text_static("settings.models.model_catalog"),
                    display_path(path),
                ));
            }
            if let Some(endpoint) = catalog.and_then(|catalog| catalog.endpoint.clone()) {
                items.push(value_item(
                    conduit::i18n::text_static("settings.models.interface"),
                    endpoint,
                ));
            }
            if let Some(detail) = catalog.and_then(|catalog| catalog.detail.clone()) {
                items.push(value_item(
                    conduit::i18n::text_static("settings.models.additional_configuration"),
                    detail,
                ));
            }
            items.extend(vec![
                SettingItem::new(
                    conduit::i18n::text_static("settings.models.model_list"),
                    self.managed_models_field(provider, view.clone()),
                )
                .layout(Axis::Vertical),
            ]);
        }
        if let Some(error) = error {
            items.push(
                SettingItem::new(
                    conduit::i18n::text_static("settings.models.read_failed"),
                    error_field(error),
                )
                .layout(Axis::Vertical)
                .keywords([conduit::i18n::text_static("settings.models.error")]),
            );
        }
        let group = settings_section(if provider == Provider::OpenCode {
            conduit::i18n::text_static("settings.models.opencode_native_models")
        } else {
            conduit::i18n::text_static("settings.models.cli_defaults")
        })
        .variant(GroupBoxVariant::Outline)
        .items(items);
        if let Some(note) = note {
            group.footer(move |_, _| note.clone())
        } else {
            group
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ConduitApp, ProviderEditor, Store};

    #[gpui_kit::test]
    fn provider_editor_tracks_fields_and_credentials_without_persisting_drafts(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
            let editor = cx.new(|cx| ProviderEditor::new(app.clone(), None, window, cx));
            let provider_count = app.read(cx).store.providers.len();
            editor.update(cx, |editor, cx| {
                assert!(editor.can_close(cx));
                editor.key.update(cx, |input, cx| {
                    input.set_value("synthetic-unsaved-key", window, cx)
                });
                assert!(!editor.can_close(cx));
                assert!(editor.confirm_discard);
                assert_eq!(app.read(cx).store.providers.len(), provider_count);
                editor
                    .key
                    .update(cx, |input, cx| input.set_value("", window, cx));
                assert!(editor.can_close(cx));
                editor.fields[3].update(cx, |input, cx| input.set_value("model-id", window, cx));
                assert!(!editor.can_close(cx));
                editor.fields[3].update(cx, |input, cx| input.set_value("", window, cx));
                editor.env_auth = !editor.initial.2;
                assert!(!editor.can_close(cx));
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn provider_editor_renders_all_fields_and_save_actions(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
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
