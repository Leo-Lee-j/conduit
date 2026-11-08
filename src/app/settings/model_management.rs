//! Editing local catalog preferences without writing native CLI configuration.
use super::*;
use crate::models::{ModelOption, preferences::ModelPreferences};

struct ModelEditor {
    app: Entity<ConduitApp>,
    engine: Provider,
    original: Option<String>,
    fields: [Entity<InputState>; 4],
    error: Option<String>,
}

impl ModelEditor {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self
            .fields
            .each_ref()
            .map(|field| field.read(cx).value().trim().to_owned());
        let context_window = if values[3].is_empty() {
            None
        } else {
            match values[3].parse::<u64>() {
                Ok(value) if value > 0 => Some(value),
                _ => {
                    self.error = Some(
                        conduit::i18n::text_static(
                            "settings.model_management.context_window_must_be_a_positive_integer",
                        )
                        .into(),
                    );
                    cx.notify();
                    return;
                }
            }
        };
        let model = ModelOption {
            id: values[0].clone(),
            label: values[1].clone(),
            note: values[2].clone(),
            context_window,
        };
        let result = self.app.update(cx, |app, cx| -> Result<(), String> {
            if !app.writable || app.closing {
                return Err(conduit::i18n::text_static(
                    "settings.model_management.unable_to_save_settings_right_now",
                )
                .into());
            }
            let previous = app.store.cli.clone();
            if let Err(error) = app.store.cli.models_mut(self.engine).save(
                self.engine,
                self.original.as_deref(),
                model,
            ) {
                app.store.cli = previous;
                return Err(error.to_string());
            }
            if !app.persist() {
                app.store.cli = previous;
                return Err(conduit::i18n::text_static(
                    "settings.model_management.failed_to_save_model_check_the_data",
                )
                .into());
            }
            cx.emit(crate::providers::ProviderCatalogChanged);
            cx.notify();
            Ok(())
        });
        match result {
            Ok(()) => window.close_dialog(cx),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}

impl Render for ModelEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = !self.app.read(cx).writable || self.app.read(cx).closing;
        let mut body = v_flex()
            .id("model-editor-body")
            .w_full()
            .gap_3()
            .max_h((window.viewport_size().height - px(210.)).max(px(140.)))
            .overflow_y_scroll()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "settings.model_management.only_app_side_model_information_is_saved",
                        &[("arg0", self.engine.label().to_string())],
                    )),
            );
        for (index, label) in [
            conduit::i18n::text_static("settings.models.model_id"),
            conduit::i18n::text_static("settings.model_management.display_name_optional"),
            conduit::i18n::text_static("settings.model_management.notes_optional"),
            conduit::i18n::text_static("settings.model_management.context_window_tokens_optional"),
        ]
        .into_iter()
        .enumerate()
        {
            body = body.child(
                v_flex().gap_1().child(label).child(
                    Input::new(&self.fields[index])
                        .disabled(disabled)
                        .aria_label(label),
                ),
            );
        }
        body = body.when_some(self.error.clone(), |body, error| {
            body.child(div().text_color(rgb(palette().danger_text)).child(error))
        });
        v_flex().gap_3().child(body).child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("model-editor-cancel")
                        .label(conduit::i18n::text_static("git_branches.cancel"))
                        .ghost()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
                .child(
                    Button::new("model-editor-save")
                        .debug_selector(|| "model-editor-save".into())
                        .label(conduit::i18n::text_static(
                            "settings.model_management.save_model",
                        ))
                        .primary()
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
    }
}

impl ConduitApp {
    pub(in crate::app) fn native_model_preferences(&self, engine: Provider) -> ModelPreferences {
        self.store
            .cli
            .engines
            .get(&engine)
            .map(|s| s.models.clone())
            .unwrap_or_default()
    }

    pub(in crate::app) fn managed_native_models(
        &self,
        engine: Provider,
        include_hidden: bool,
    ) -> Vec<ModelOption> {
        let preferences = self.native_model_preferences(engine);
        let native = self
            .engine_catalog(engine)
            .map(|catalog| catalog.models.as_slice())
            .unwrap_or_default();
        let mut models = preferences.merge(native, include_hidden);
        // A persisted default remains recognizable even after hiding or catalog removal.
        if !include_hidden
            && let Some(defaults) = self.store.session_defaults.get(&engine)
            && defaults.channel == ModelChannel::Cli
            && let Some(id) = &defaults.model
            && !models.iter().any(|m| &m.id == id)
        {
            let model = preferences
                .merge(native, true)
                .into_iter()
                .find(|m| &m.id == id)
                .unwrap_or_else(|| ModelOption {
                    id: id.clone(),
                    label: id.clone(),
                    note: conduit::i18n::text_static(
                        "settings.model_management.new_session_default_not_found_in_catalog",
                    )
                    .into(),
                    context_window: None,
                });
            models.push(model);
        }
        models
    }

    fn open_model_editor(
        &mut self,
        engine: Provider,
        model: Option<ModelOption>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let app = cx.entity();
        let values = model
            .as_ref()
            .map(|m| {
                [
                    m.id.clone(),
                    m.label.clone(),
                    m.note.clone(),
                    m.context_window.map(|n| n.to_string()).unwrap_or_default(),
                ]
            })
            .unwrap_or_default();
        let editor = cx.new(|cx| ModelEditor {
            app,
            engine,
            original: model.map(|m| m.id),
            error: None,
            fields: std::array::from_fn(|index| {
                cx.new(|cx| InputState::new(window, cx).default_value(values[index].clone()))
            }),
        });
        window.open_dialog(cx, move |dialog, window, _| {
            let editor = editor.clone();
            dialog
                .title(conduit::i18n::text_static(
                    "settings.model_management.manage_cli_models",
                ))
                .margin_top(px(32.))
                .overlay_closable(false)
                .w((window.viewport_size().width - px(60.)).clamp(px(0.), px(620.)))
                .content(move |body, _, _| body.child(editor.clone()))
        });
    }

    fn change_model_preferences(
        &mut self,
        engine: Provider,
        change: impl FnOnce(&mut ModelPreferences),
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let previous = self.store.cli.clone();
        change(self.store.cli.models_mut(engine));
        if !self.persist() {
            self.store.cli = previous;
        }
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
    }

    pub(super) fn managed_models_field(
        &self,
        engine: Provider,
        view: Entity<Self>,
    ) -> SettingField<SharedString> {
        let rows = self.managed_native_models(engine, true);
        let preferences = self.native_model_preferences(engine);
        let default = self
            .store
            .session_defaults
            .get(&engine)
            .filter(|d| d.channel == ModelChannel::Cli)
            .and_then(|d| d.model.clone());
        let native_default = self.engine_catalog(engine).and_then(|c| c.current.clone());
        let disabled = !self.writable || self.closing;
        let search = self.connection_settings.model_search.clone();
        let limit = self.connection_settings.model_limit;
        SettingField::render(move |_, _, cx| {
            let query = search.read(cx).value().to_lowercase();
            let matches = rows
                .iter()
                .filter(|model| {
                    let text =
                        format!("{} {} {}", model.id, model.label, model.note).to_lowercase();
                    query.split_whitespace().all(|word| text.contains(word))
                })
                .collect::<Vec<_>>();
            let add = view.clone();
            let mut list = v_flex().w_full().gap_2().child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static("settings.model_management.hiding_only_affects_the_candidate_list_current")),
                    )
                    .child(
                        Button::new("model-add")
                            .debug_selector(|| "model-add".into())
                            .small()
                            .outline()
                            .label(conduit::i18n::text_static("settings.model_management.add_model"))
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                add.update(cx, |app, cx| {
                                    app.open_model_editor(engine, None, window, cx)
                                })
                            }),
                    ),
            );
            list = list
                .child(
                    Input::new(&search)
                        .small()
                        .aria_label(conduit::i18n::text_static(
                            "settings.model_management.search_cli_models",
                        )),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::format(
                            "settings.model_management.showing_matching_models_definitions_total",
                            &[
                                ("arg0", format!("{}", matches.len().min(limit))),
                                ("arg1", format!("{}", matches.len())),
                                ("arg2", format!("{}", rows.len())),
                            ],
                        )),
                );
            if rows.is_empty() {
                list = list.child(conduit::i18n::text_static(
                    "settings.model_management.no_models_found_add_a_model_id",
                ));
            } else if matches.is_empty() {
                list = list.child(conduit::i18n::text_static(
                    "settings.model_management.no_matching_models_adjust_the_search",
                ));
            }
            for (index, model) in matches.iter().take(limit).copied().enumerate() {
                let hidden = preferences.hidden.contains(&model.id);
                let custom = preferences.custom.iter().any(|m| m.id == model.id);
                let current = default.as_deref() == Some(&model.id);
                let mut badges = vec![if custom {
                    conduit::i18n::text_static("settings.model_management.local_definition")
                } else {
                    conduit::i18n::text_static("settings.model_management.cli_source")
                }];
                if hidden {
                    badges.push(conduit::i18n::text_static(
                        "settings.model_management.hidden",
                    ));
                }
                if current {
                    badges.push(conduit::i18n::text_static(
                        "settings.defaults.new_session_defaults",
                    ));
                }
                if native_default.as_deref() == Some(&model.id) {
                    badges.push(conduit::i18n::text_static("generation.cli_default"));
                }
                let edit = view.clone();
                let edit_model = model.clone();
                let hide = view.clone();
                let hide_id = model.id.clone();
                let remove = view.clone();
                let remove_id = model.id.clone();
                let use_view = view.clone();
                let use_id = model.id.clone();
                list = list.child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .items_start()
                                .child(model_mark(engine, Some(&model.id)))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(ui_text_size(13.))
                                                .font_semibold()
                                                .child(model.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(12.))
                                                .text_color(rgb(palette().muted))
                                                .child(model.id.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(11.))
                                                .text_color(rgb(palette().muted))
                                                .child(badges.join(" · ")),
                                        )
                                        .when(!model.note.is_empty(), |row| {
                                            row.child(
                                                div()
                                                    .text_size(ui_text_size(12.))
                                                    .child(model.note.clone()),
                                            )
                                        })
                                        .when_some(model.context_window, |row, tokens| {
                                            row.child(div().text_size(ui_text_size(11.)).child(
                                                conduit::i18n::format(
                                                    "settings.model_management.context_tokens",
                                                    &[("tokens", format!("{}", tokens))],
                                                ),
                                            ))
                                        }),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new(("model-default", index))
                                        .small()
                                        .outline()
                                        .label(conduit::i18n::text_static(
                                            "settings.models.use_for_new_sessions",
                                        ))
                                        .disabled(disabled || current)
                                        .on_click(move |_, _, cx| {
                                            use_view.update(cx, |app, cx| {
                                                if !app.writable || app.closing {
                                                    return;
                                                }
                                                let previous = app.store.session_defaults.clone();
                                                app.store.session_defaults.insert(
                                                    engine,
                                                    crate::store::SessionDefaults {
                                                        channel: ModelChannel::Cli,
                                                        model: Some(use_id.clone()),
                                                    },
                                                );
                                                if !app.persist() {
                                                    app.store.session_defaults = previous;
                                                }
                                                cx.emit(crate::providers::ProviderCatalogChanged);
                                                cx.notify();
                                            })
                                        }),
                                )
                                .child(
                                    Button::new(("model-edit", index))
                                        .small()
                                        .ghost()
                                        .label(conduit::i18n::text_static(
                                            "settings.extensions.edit",
                                        ))
                                        .disabled(disabled)
                                        .on_click(move |_, window, cx| {
                                            edit.update(cx, |app, cx| {
                                                app.open_model_editor(
                                                    engine,
                                                    Some(edit_model.clone()),
                                                    window,
                                                    cx,
                                                )
                                            })
                                        }),
                                )
                                .child(
                                    Button::new(("model-hide", index))
                                        .debug_selector(move || format!("model-hide-{index}"))
                                        .small()
                                        .ghost()
                                        .label(if hidden {
                                            conduit::i18n::text_static(
                                                "settings.model_management.show_again",
                                            )
                                        } else {
                                            conduit::i18n::text_static(
                                                "settings.model_management.hide",
                                            )
                                        })
                                        .disabled(disabled)
                                        .on_click(move |_, _, cx| {
                                            hide.update(cx, |app, cx| {
                                                app.change_model_preferences(
                                                    engine,
                                                    |prefs| {
                                                        if hidden {
                                                            prefs.hidden.remove(&hide_id);
                                                        } else {
                                                            prefs.hidden.insert(hide_id.clone());
                                                        }
                                                    },
                                                    cx,
                                                )
                                            })
                                        }),
                                )
                                .when(custom, |row| {
                                    row.child(
                                        Button::new(("model-remove", index))
                                            .small()
                                            .ghost()
                                            .label(conduit::i18n::text_static(
                                                "settings.model_management.remove_local_definition",
                                            ))
                                            .disabled(disabled)
                                            .on_click(move |_, _, cx| {
                                                remove.update(cx, |app, cx| {
                                                    app.change_model_preferences(
                                                        engine,
                                                        |prefs| {
                                                            prefs
                                                                .custom
                                                                .retain(|m| m.id != remove_id)
                                                        },
                                                        cx,
                                                    )
                                                })
                                            }),
                                    )
                                }),
                        ),
                );
            }
            if matches.len() > limit {
                let more = view.clone();
                list = list.child(
                    Button::new("models-load-more")
                        .small()
                        .outline()
                        .label(conduit::i18n::text_static(
                            "settings.model_management.show_50_more_models",
                        ))
                        .on_click(move |_, _, cx| {
                            more.update(cx, |app, cx| {
                                app.connection_settings.model_limit += 50;
                                cx.notify();
                            })
                        }),
                );
            }
            list
        })
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{
        ConduitApp, InputState, ModelChannel, ModelEditor, Provider, Store, SystemTime, UNIX_EPOCH,
        set_palette,
    };
    use gpui_kit::{AppContext as _, TestAppContext, px, size};

    #[gpui_kit::test]
    fn settings_model_editor_validates_persists_and_respects_readonly(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let root = std::env::temp_dir().join(format!(
            "conduit-model-management-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = root.join("state.json");
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app =
                    ConduitApp::with_store(window, cx, Ok(Store::default()), false, false);
                app.persistence = crate::persistence::Persistence::new(state.clone());
                app.connection_settings.syncing = true;
                app.catalogs.clear();
                app
            });
            let editor = cx.new(|cx| ModelEditor {
                app: app.clone(),
                engine: Provider::Codex,
                original: None,
                error: None,
                fields: std::array::from_fn(|_| cx.new(|cx| InputState::new(window, cx))),
            });
            *output.borrow_mut() = Some((app, editor.clone()));
            gpui_kit::component::Root::new(editor, window, cx)
        });
        let (app, editor) = slot.borrow().as_ref().unwrap().clone();
        for (width, height, mode) in [
            (850., 500., gpui_kit::component::ThemeMode::Light),
            (1280., 900., gpui_kit::component::ThemeMode::Dark),
        ] {
            cx.simulate_resize(size(px(width), px(height)));
            cx.update(|window, cx| {
                set_palette(mode);
                gpui_kit::component::Theme::change(mode, Some(window), cx);
                let _ = window.draw(cx);
            });
            cx.run_until_parked();
            let save = cx
                .debug_bounds("model-editor-save")
                .expect("save remains reachable");
            assert!(save.bottom() <= px(height));
        }
        cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                editor.save(window, cx);
                assert!(editor.error.is_some());
                editor.fields[0]
                    .update(cx, |field, cx| field.set_value("custom-model", window, cx));
                editor.fields[1]
                    .update(cx, |field, cx| field.set_value("Friendly name", window, cx));
                editor.fields[3].update(cx, |field, cx| field.set_value("64000", window, cx));
                editor.save(window, cx);
            })
        });
        cx.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.persistence.flush();
                assert!(app.persistence.error.is_none());
                let loaded: Store =
                    serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
                assert_eq!(
                    loaded.cli.engines[&Provider::Codex].models.custom[0].context_window,
                    Some(64000)
                );
                assert_eq!(
                    app.channel_models(Provider::Codex, &ModelChannel::Cli).1[0].label,
                    "Friendly name"
                );
                app.change_model_preferences(
                    Provider::Codex,
                    |prefs| {
                        prefs.hidden.insert("custom-model".into());
                    },
                    cx,
                );
                assert!(
                    app.channel_models(Provider::Codex, &ModelChannel::Cli)
                        .1
                        .is_empty()
                );
                app.store.session_defaults.insert(
                    Provider::Codex,
                    crate::store::SessionDefaults {
                        channel: ModelChannel::Cli,
                        model: Some("custom-model".into()),
                    },
                );
                assert_eq!(
                    app.channel_models(Provider::Codex, &ModelChannel::Cli)
                        .1
                        .len(),
                    1
                );
                app.writable = false;
                app.change_model_preferences(Provider::Codex, |prefs| prefs.hidden.clear(), cx);
                assert!(
                    app.native_model_preferences(Provider::Codex)
                        .hidden
                        .contains("custom-model")
                );
                app.persistence.flush();
            })
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
