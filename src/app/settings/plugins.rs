//! Plugin management is separate from CLI Skills and MCP services.
use super::*;
use gpui_kit::component::switch::Switch;
mod catalog;
#[cfg(all(test, feature = "ui-tests"))]
mod encourager_tests;
mod provider_picker;

fn pet_setting_row(
    id: &'static str,
    label: &'static str,
    control: impl IntoElement,
) -> gpui_kit::Stateful<Div> {
    h_flex()
        .id(id)
        .debug_selector(move || id.into())
        .w_full()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .px_3()
        .py_1()
        .child(
            div()
                .flex_1()
                .min_w(px(160.))
                .text_size(ui_text_size(13.))
                .child(conduit::i18n::text_static(label)),
        )
        .child(control)
}

fn pet_picker_control(control: impl IntoElement) -> Div {
    div().w(px(288.)).max_w_full().min_w_0().child(control)
}

fn pet_switch_control(control: impl IntoElement) -> Div {
    h_flex().h(px(32.)).justify_end().child(control)
}
fn text(key: &str) -> String {
    conduit::i18n::text(&format!("settings.plugins.{key}"))
}
fn caption(value: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(12.))
        .line_height(relative(1.5))
        .text_color(rgb(palette().muted))
        .child(value.into())
}
impl ConduitApp {
    fn settings_installed_plugins_content(&self, view: Entity<Self>) -> AnyElement {
        let state = &self.plugins.packages;
        let disabled = !self.writable || self.closing || state.busy;
        let install_view = view.clone();
        let refresh_view = view.clone();
        let default_view = view.clone();
        let default_active = self.store.ui.plugins.file_icon_theme.is_none();
        let mut content = settings_scroll_area("settings-plugins-list")
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("plugin-install")
                            .debug_selector(|| "plugin-install".into())
                            .icon(FullIconName::Plus)
                            .label(text("install"))
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                install_view
                                    .update(cx, |app, cx| app.install_plugin_folder(window, cx))
                            }),
                    )
                    .child(
                        Button::new("plugin-refresh")
                            .debug_selector(|| "plugin-refresh".into())
                            .icon(IconName::RefreshCw)
                            .label(text("refresh"))
                            .disabled(state.busy || self.closing)
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |app, cx| app.refresh_installed_plugins(cx))
                            }),
                    )
                    .child(
                        Button::new("plugin-folder")
                            .label(text("open_folder"))
                            .ghost()
                            .disabled(disabled)
                            .on_click(|_, _, cx| {
                                let root = crate::app::plugin_ui::management::installation_root();
                                if std::fs::create_dir_all(&root).is_ok() {
                                    cx.reveal_path(&root);
                                }
                            }),
                    ),
            )
            .when_some(state.feedback.clone(), |list, message| {
                list.child(caption(message))
            })
            .child(
                v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(FullIconName::Blocks).small())
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_medium()
                                    .child(text("core")),
                            )
                            .child(caption(text("built_in"))),
                    )
                    .child(caption(text("core_description"))),
            )
            .child(self.settings_encourager_content(view.clone()))
            .child(
                v_flex()
                    .w_full()
                    .gap_3()
                    .refine_style(&settings_group_style())
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .font_medium()
                            .child(text("default_icons")),
                    )
                    .child(caption(text("default_description")))
                    .child(
                        Button::new("plugin-default-icons")
                            .debug_selector(|| "plugin-default-icons".into())
                            .label(text(if default_active {
                                "active"
                            } else {
                                "use_icons"
                            }))
                            .selected(default_active)
                            .disabled(disabled || default_active)
                            .on_click(move |_, _, cx| {
                                default_view
                                    .update(cx, |app, cx| app.select_file_icon_theme(None, cx))
                            }),
                    ),
            );
        if state.busy && !state.initialized {
            content = content.child(caption(text("loading")));
        }
        for entry in &state.entries {
            let manifest = &entry.package.manifest;
            let enabled = entry.registration.is_some();
            let active =
                self.store.ui.plugins.file_icon_theme.as_deref() == Some(&manifest.id) && enabled;
            let id = manifest.id.clone();
            let toggle_id = id.clone();
            let select_id = id.clone();
            let uninstall_id = id.clone();
            let toggle_view = view.clone();
            let select_view = view.clone();
            let uninstall_view = view.clone();
            let name = manifest.name.clone();
            content = content.child(
                v_flex()
                    .id(SharedString::from(format!("plugin-{id}")))
                    .debug_selector(move || format!("plugin-{id}"))
                    .refine_style(&settings_group_style())
                    .gap_3()
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_medium()
                                    .child(manifest.name.clone()),
                            )
                            .child(caption(manifest.version.clone()))
                            .child(caption(text(if enabled { "enabled" } else { "disabled" }))),
                    )
                    .child(caption(manifest.description.clone()))
                    .child(caption(format!("{} · {}", text("file_icons"), manifest.id)))
                    .when_some(entry.package.source.as_ref(), |card, source| {
                        card.child(caption(format!("GitHub · {}", source.repository())))
                    })
                    .child(
                        h_flex().gap_3().children(
                            [
                                Path::new("src"),
                                Path::new("main.rs"),
                                Path::new("settings.json"),
                                Path::new("README.md"),
                            ]
                            .into_iter()
                            .map(|path| {
                                // Preview this package without changing the user's active theme.
                                let icon = manifest
                                    .file_icons
                                    .resolve(path, path == Path::new("src"), false)
                                    .and_then(|key| entry.package.assets.get(key))
                                    .map(|bytes| Icon::default().data(bytes))
                                    .unwrap_or_else(|| Icon::new(IconName::File));
                                icon.size(px(20.))
                                    .text_color(rgb(palette().muted))
                                    .into_any_element()
                            }),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("plugin-use-icons")
                                    .debug_selector({
                                        let id = select_id.clone();
                                        move || format!("plugin-use-{id}")
                                    })
                                    .label(text(if active { "active" } else { "use_icons" }))
                                    .selected(active)
                                    .disabled(disabled || !enabled || active)
                                    .on_click(move |_, _, cx| {
                                        select_view.update(cx, |app, cx| {
                                            app.select_file_icon_theme(Some(select_id.clone()), cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new("plugin-toggle")
                                    .debug_selector({
                                        let id = toggle_id.clone();
                                        move || format!("plugin-toggle-{id}")
                                    })
                                    .label(text(if enabled { "disable" } else { "enable" }))
                                    .disabled(disabled)
                                    .on_click(move |_, _, cx| {
                                        toggle_view.update(cx, |app, cx| {
                                            app.set_plugin_enabled(&toggle_id, !enabled, cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new("plugin-uninstall")
                                    .debug_selector({
                                        let id = uninstall_id.clone();
                                        move || format!("plugin-uninstall-{id}")
                                    })
                                    .label(text("uninstall"))
                                    .ghost()
                                    .disabled(disabled)
                                    .on_click(move |_, window, cx| {
                                        let view = uninstall_view.clone();
                                        let id = uninstall_id.clone();
                                        let name = name.clone();
                                        window.open_dialog(cx, move |dialog, _, _| {
                                            let view = view.clone();
                                            let id = id.clone();
                                            dialog
                                                .title(text("confirm_title"))
                                                .child(caption(conduit::i18n::format(
                                                    "settings.plugins.confirm_body",
                                                    &[("name", name.clone())],
                                                )))
                                                .footer(
                                                    h_flex()
                                                        .justify_end()
                                                        .gap_2()
                                                        .child(
                                                            Button::new("plugin-uninstall-cancel")
                                                                .label(text("cancel"))
                                                                .on_click(|_, window, cx| {
                                                                    window.close_dialog(cx)
                                                                }),
                                                        )
                                                        .child(
                                                            Button::new("plugin-uninstall-confirm")
                                                                .debug_selector(|| {
                                                                    "plugin-uninstall-confirm"
                                                                        .into()
                                                                })
                                                                .label(text("uninstall"))
                                                                .on_click(move |_, window, cx| {
                                                                    view.update(cx, |app, cx| {
                                                                        app.uninstall_plugin(
                                                                            id.clone(),
                                                                            cx,
                                                                        )
                                                                    });
                                                                    window.close_dialog(cx);
                                                                }),
                                                        ),
                                                )
                                        });
                                    }),
                            ),
                    ),
            );
        }
        if state.entries.is_empty() && !state.busy {
            content = content.child(
                v_flex()
                    .py_4()
                    .gap_2()
                    .child(div().text_size(ui_text_size(14.)).child(text("empty"))),
            );
        }
        if !state.errors.is_empty() {
            content = content.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_color(rgb(palette().error_text))
                            .child(text("errors")),
                    )
                    .children(state.errors.iter().map(|error| caption(error.clone()))),
            );
        }
        content.into_any_element()
    }

    fn settings_encourager_content(&self, view: Entity<Self>) -> AnyElement {
        use crate::store::{EncouragerPersona, EncouragerVerbosity};
        let settings = &self.store.ui.encourager;
        let enabled = settings.enabled;
        let pet_enabled = settings.pet_enabled;
        let disabled = !self.writable || self.closing;
        let toggle_view = view.clone();
        let toggle = Switch::new("encourager-plugin-toggle")
            .checked(enabled)
            .accessibility_label(conduit::i18n::text_static("encourager.enable"))
            .disabled(disabled)
            .on_click(move |value, window, cx| {
                toggle_view.update(cx, |app, cx| app.set_encourager_enabled(*value, window, cx));
            });
        let pet_view = view.clone();
        let pet_toggle = Switch::new("encourager-pet-toggle")
            .checked(pet_enabled)
            .accessibility_label(conduit::i18n::text_static("encourager.pet.disabled"))
            .disabled(disabled || !enabled)
            .on_click(move |value, _, cx| {
                pet_view.update(cx, |app, cx| {
                    app.set_encourager_pet_enabled(*value, cx);
                });
            });

        let providers = provider_picker::choices(&self.store.providers);
        let selected_provider = settings.provider_id.clone();
        let provider_picker = provider_picker::ProviderPicker {
            app: view.clone(),
            choices: providers.clone(),
            selected: selected_provider.clone(),
            disabled,
        };

        let model_choices: Vec<_> = selected_provider
            .as_ref()
            .and_then(|id| providers.iter().find(|provider| &provider.id == id))
            .map(|provider| provider.models.clone())
            .unwrap_or_default();
        let model_label = settings
            .model
            .as_ref()
            .filter(|model| model_choices.contains(model))
            .cloned()
            .unwrap_or_else(|| conduit::i18n::text("encourager.no_model"));
        let model_view = view.clone();
        let current_model = settings.model.clone();
        let model_picker = Button::new("encourager-model")
            .debug_selector(|| "encourager-model".into())
            .outline()
            .h(px(32.))
            .w_full()
            .dropdown_caret(true)
            .accessibility_label(conduit::i18n::text_static("encourager.setting.model"))
            .label(model_label)
            .disabled(disabled || model_choices.is_empty())
            .dropdown_menu(move |mut menu, _, _| {
                for model in &model_choices {
                    let model = model.clone();
                    let view = model_view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(model.clone())
                            .checked(current_model.as_ref() == Some(&model))
                            .on_click(move |_, _, cx| {
                                view.update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return;
                                    }
                                    app.reset_encourager_output();
                                    app.store.ui.encourager.model = Some(model.clone());
                                    app.persist();
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });

        let persona_label = conduit::i18n::text_static(settings.persona.label());
        let persona_view = view.clone();
        let persona_picker = Button::new("encourager-persona-setting")
            .debug_selector(|| "encourager-persona-setting".into())
            .outline()
            .h(px(32.))
            .w_full()
            .dropdown_caret(true)
            .accessibility_label(conduit::i18n::text_static("encourager.setting.persona"))
            .label(persona_label)
            .disabled(disabled)
            .dropdown_menu(move |mut menu, _, _| {
                for persona in EncouragerPersona::ALL {
                    let view = persona_view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(conduit::i18n::text_static(persona.label())).on_click(
                            move |_, _, cx| {
                                view.update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return;
                                    }
                                    app.reset_encourager_output();
                                    app.store.ui.encourager.persona = persona;
                                    app.persist();
                                    cx.notify();
                                });
                            },
                        ),
                    );
                }
                menu
            });

        let verbosity_label = conduit::i18n::text_static(settings.verbosity.label());
        let verbosity_view = view.clone();
        let verbosity_picker = Button::new("encourager-verbosity")
            .debug_selector(|| "encourager-verbosity".into())
            .outline()
            .h(px(32.))
            .w_full()
            .dropdown_caret(true)
            .accessibility_label(conduit::i18n::text_static("encourager.setting.verbosity"))
            .label(verbosity_label)
            .disabled(disabled)
            .dropdown_menu(move |mut menu, _, _| {
                for verbosity in EncouragerVerbosity::ALL {
                    let view = verbosity_view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(conduit::i18n::text_static(verbosity.label())).on_click(
                            move |_, _, cx| {
                                view.update(cx, |app, cx| {
                                    if !app.writable || app.closing {
                                        return;
                                    }
                                    app.reset_encourager_output();
                                    app.store.ui.encourager.verbosity = verbosity;
                                    app.persist();
                                    cx.notify();
                                });
                            },
                        ),
                    );
                }
                menu
            });

        let include_context = settings.include_context;
        let context_view = view;
        let context_button = Switch::new("encourager-include-context")
            .checked(include_context)
            .accessibility_label(conduit::i18n::text_static("encourager.setting.context"))
            .disabled(disabled)
            .on_click(move |value, _, cx| {
                context_view.update(cx, |app, cx| {
                    if !app.writable || app.closing {
                        return;
                    }
                    app.reset_encourager_output();
                    app.store.ui.encourager.include_context = *value;
                    app.persist();
                    cx.notify();
                });
            });

        v_flex()
            .id("programmer-encourager-settings")
            .debug_selector(|| "programmer-encourager-settings".into())
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(FullIconName::Cat).small())
                    .child(
                        div()
                            .font_medium()
                            .text_size(ui_text_size(14.))
                            .child(conduit::i18n::text_static("encourager.title")),
                    ),
            )
            .child(
                v_flex()
                    .w_full()
                    .refine_style(&settings_group_style())
                    .px_0()
                    .py_2()
                    .gap_1()
                    .child(pet_setting_row(
                        "encourager-enable-row",
                        "encourager.enable",
                        pet_switch_control(toggle),
                    ))
                    .child(pet_setting_row(
                        "encourager-pet-row",
                        "encourager.pet.disabled",
                        pet_switch_control(pet_toggle),
                    ))
                    .child(pet_setting_row(
                        "encourager-service-row",
                        "encourager.setting.service",
                        pet_picker_control(provider_picker),
                    ))
                    .when(providers.is_empty(), |this| {
                        this.child(
                            caption(conduit::i18n::text_static(
                                "encourager.no_compatible_services",
                            ))
                            .id("encourager-service-hint")
                            .debug_selector(|| "encourager-service-hint".into())
                            .px_3(),
                        )
                    })
                    .child(pet_setting_row(
                        "encourager-model-row",
                        "encourager.setting.model",
                        pet_picker_control(model_picker),
                    ))
                    .child(pet_setting_row(
                        "encourager-persona-row",
                        "encourager.setting.persona",
                        pet_picker_control(persona_picker),
                    ))
                    .child(pet_setting_row(
                        "encourager-verbosity-row",
                        "encourager.setting.verbosity",
                        pet_picker_control(verbosity_picker),
                    ))
                    .child(pet_setting_row(
                        "encourager-context-row",
                        "encourager.setting.context",
                        pet_switch_control(context_button),
                    ))
                    .when(include_context, |section| {
                        section.child(
                            caption(conduit::i18n::text_static(
                                "encourager.setting.privacy_context_on",
                            ))
                            .px_3(),
                        )
                    }),
            )
            .into_any_element()
    }
}
