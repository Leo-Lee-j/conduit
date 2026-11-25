use super::list::{service_avatar, service_badge};
use super::*;

impl ConduitApp {
    pub(super) fn service_details(
        &self,
        engine: Provider,
        channel: &ModelChannel,
        tab: DetailTab,
        view: Entity<Self>,
    ) -> AnyElement {
        let provider = match channel {
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine),
            _ => None,
        };
        if matches!(channel, ModelChannel::Provider(_)) && provider.is_none() {
            return div()
                .p_4()
                .child(service_text(
                    "settings.defaults.service_unavailable_select_another_service",
                ))
                .into_any_element();
        }
        let native = *channel == ModelChannel::Cli;
        let imported = provider.is_some_and(|p| matches!(p.source, ProviderSource::Ccs { .. }));
        let current = self.default_selection(engine).channel == *channel;
        let label = if native {
            service_text("settings.defaults.native_cli_configuration").to_string()
        } else {
            self.channel_label(engine, channel)
        };
        let model_count = if native {
            self.managed_native_models(engine, true).len()
        } else {
            provider.map(|p| p.catalog().len()).unwrap_or_default()
        };
        let mut header = h_flex()
            .debug_selector(|| "service-detail-header".into())
            .w_full()
            .items_start()
            .justify_between()
            .flex_wrap()
            .gap_3()
            .p_3()
            .child(
                h_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .items_start()
                    .gap_3()
                    .child(service_avatar(&label, true))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .flex_wrap()
                                    .child(
                                        div()
                                            .font_semibold()
                                            .text_size(ui_text_size(16.))
                                            .child(label),
                                    )
                                    .when(current, |row| {
                                        row.child(service_badge(service_text(
                                            "settings.services.current_default",
                                        )))
                                    }),
                            )
                            .when_some(provider, |description, provider| {
                                description.child(
                                    div()
                                        .w_full()
                                        .truncate()
                                        .text_size(ui_text_size(11.))
                                        .text_color(rgb(palette().muted))
                                        .child(provider.base_url.clone()),
                                )
                            })
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(service_text(if imported {
                                        "settings.services.imported_readonly"
                                    } else if native {
                                        "settings.services.native_catalog"
                                    } else {
                                        "settings.services.edit_here"
                                    })),
                            ),
                    ),
            );
        header = header.child(self.service_detail_actions(engine, channel, view.clone()));
        let mut detail = v_flex()
            .id("service-details")
            .test_support()
            .debug_selector(|| "service-details".into())
            .w_full()
            .min_w_0()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(header);
        if native && engine.descriptor().capabilities.model_services {
            let mut tabs = h_flex().w_full().px_3().pb_2().gap_2().flex_wrap();
            for (index, (candidate, title)) in [
                (
                    DetailTab::Models,
                    service_text("settings.services.native_catalog"),
                ),
                (
                    DetailTab::Sources,
                    service_text("settings.models.configuration_sources"),
                ),
            ]
            .into_iter()
            .enumerate()
            {
                let select = view.clone();
                tabs = tabs.child(
                    Button::new(("service-detail-tab", index))
                        .debug_selector(move || format!("service-detail-tab-{index}"))
                        .ghost()
                        .small()
                        .label(title)
                        .selected(tab == candidate)
                        .on_click(move |_, _, cx| {
                            select.update(cx, |app, cx| {
                                app.connection_settings.service_tab = candidate;
                                cx.notify();
                            })
                        }),
                );
            }
            detail = detail.child(tabs);
        }
        let models = if native && tab == DetailTab::Sources {
            detail_settings(
                "service-source-settings",
                self.models_source_group(engine, view.clone()),
            )
        } else if native {
            detail_settings(
                "service-native-model-settings",
                self.models_provider_group(engine, view.clone()),
            )
        } else {
            self.service_models_detail(engine, channel, provider, view.clone())
        };
        detail
            .child(
                v_flex()
                    .debug_selector(|| "service-models-section".into())
                    .w_full()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        service_section_heading(
                            service_text(if native && tab == DetailTab::Sources {
                                "settings.models.configuration_sources"
                            } else {
                                "settings.models.model_list"
                            }),
                            (!(native && tab == DetailTab::Sources)).then_some(model_count),
                        )
                        .debug_selector(|| "service-models-heading".into()),
                    )
                    .child(models),
            )
            .child(
                v_flex()
                    .id("service-sessions-section")
                    .debug_selector(|| "service-sessions-section".into())
                    .w_full()
                    .gap_2()
                    .p_3()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        service_section_heading(
                            service_text("settings.services.sessions"),
                            Some(self.service_session_count(engine, channel)),
                        )
                        .debug_selector(|| "service-sessions-heading".into()),
                    )
                    .child(self.service_sessions_detail(engine, channel, provider)),
            )
            .into_any_element()
    }

    fn service_models_detail(
        &self,
        engine: Provider,
        channel: &ModelChannel,
        provider: Option<&ModelProvider>,
        view: Entity<Self>,
    ) -> AnyElement {
        ServiceModels {
            engine,
            channel: channel.clone(),
            models: provider.map(|p| p.catalog()).unwrap_or_default(),
            disabled: !self.writable || self.closing || self.connection_settings.saving,
            defaults: self.default_selection(engine),
            search: self.connection_settings.model_search.clone(),
            view,
        }
        .into_any_element()
    }

    fn service_sessions_detail(
        &self,
        engine: Provider,
        channel: &ModelChannel,
        provider: Option<&ModelProvider>,
    ) -> AnyElement {
        let session_count = self.service_session_count(engine, channel);
        let mut list = v_flex().w_full().gap_1();
        if session_count == 0 {
            list = list.child(
                div()
                    .py_1()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(service_text("settings.services.no_sessions")),
            );
        }
        for workspace in &self.store.workspaces {
            for session in workspace
                .sessions
                .iter()
                .filter(|s| s.provider == engine && s.channel == *channel)
            {
                list = list.child(
                    v_flex()
                        .w_full()
                        .py_2()
                        .gap_1()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .child(session.title.clone()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(workspace.name()),
                        ),
                );
            }
        }
        if let Some(provider) = &provider {
            let uses = self.provider_configuration_references(&provider.id);
            if !uses.is_empty() {
                list = list
                    .child(
                        div()
                            .pt_2()
                            .font_medium()
                            .text_size(ui_text_size(13.))
                            .child(service_text("settings.services.other_uses")),
                    )
                    .children(uses.into_iter().map(|label| {
                        div()
                            .py_2()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(label)
                    }));
            }
        }
        list.into_any_element()
    }

    fn service_detail_actions(
        &self,
        engine: Provider,
        channel: &ModelChannel,
        view: Entity<Self>,
    ) -> AnyElement {
        let channel = channel.clone();
        let provider = match &channel {
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine)
                .cloned(),
            _ => None,
        };
        let imported = provider
            .as_ref()
            .is_some_and(|p| matches!(p.source, ProviderSource::Ccs { .. }));
        let disabled = !self.writable || self.closing || self.connection_settings.saving;
        let defaults = self.default_selection(engine);
        let current = defaults.channel == channel;
        let use_view = view.clone();
        let use_channel = channel;
        let mut footer = h_flex()
            .flex_shrink_0()
            .justify_end()
            .gap_2()
            .flex_wrap()
            .items_start();
        if let Some(provider) = provider {
            if imported {
                footer = footer.child(
                    Button::new("service-edit-cc-switch")
                        .small()
                        .outline()
                        .icon(FullIconName::ExternalLink)
                        .label(service_text("settings.services.edit_cc_switch"))
                        .disabled(self.closing)
                        .tooltip(service_text("settings.services.cc_switch_hint"))
                        .on_click(|_, window, cx| {
                            if !launch_cc_switch() {
                                window.push_notification(
                                    Notification::error(service_text(
                                        "settings.services.cc_switch_unavailable",
                                    )),
                                    cx,
                                );
                            }
                        }),
                );
            } else {
                let edit_view = view.clone();
                let id = provider.id.clone();
                let delete_id = id.clone();
                let edit_id = id;
                let delete_view = view.clone();
                let in_use = !self.provider_references(&provider.id).is_empty();
                footer = footer.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("service-edit-local")
                                .debug_selector(|| "service-edit-local".into())
                                .small()
                                .outline()
                                .label(service_text("settings.extensions.edit"))
                                .disabled(disabled)
                                .on_click(move |_, window, cx| {
                                    edit_view.update(cx, |app, cx| {
                                        let provider = app
                                            .store
                                            .providers
                                            .iter()
                                            .find(|p| {
                                                p.id == edit_id && p.source == ProviderSource::Local
                                            })
                                            .cloned();
                                        if let Some(provider) = provider {
                                            app.open_provider_editor(Some(provider), window, cx);
                                        }
                                    })
                                }),
                        )
                        .child(
                            Button::new("service-delete-local")
                                .debug_selector(|| "service-delete-local".into())
                                .small()
                                .ghost()
                                .icon(FullIconName::Trash)
                                .accessibility_label(service_text(
                                    "settings.models.delete_provider",
                                ))
                                .tooltip(service_text(if in_use {
                                    "settings.models.first_change_the_sessions_naming_settings_or"
                                } else {
                                    "settings.models.delete_provider"
                                }))
                                .disabled(disabled || in_use)
                                .on_click(move |_, window, cx| {
                                    confirm_delete(
                                        delete_view.clone(),
                                        delete_id.clone(),
                                        window,
                                        cx,
                                    )
                                }),
                        ),
                );
            }
        }
        footer = footer.when(!current, |footer| {
            footer.child(
                Button::new("service-detail-default")
                    .debug_selector(|| "service-detail-default".into())
                    .small()
                    .primary()
                    .label(service_text(if current {
                        "settings.services.current_default"
                    } else {
                        "settings.services.set_new_default"
                    }))
                    .disabled(disabled || current)
                    .tooltip(service_text("settings.services.switch_default_hint"))
                    .on_click(move |_, window, cx| {
                        use_view.update(cx, |app, cx| {
                            app.set_service_default(engine, use_channel.clone(), None, window, cx);
                        })
                    }),
            )
        });

        footer.into_any_element()
    }
}

fn service_section_heading(label: &'static str, count: Option<usize>) -> Div {
    h_flex()
        .gap_2()
        .child(
            div()
                .font_semibold()
                .text_size(ui_text_size(12.))
                .child(label),
        )
        .when_some(count, |heading, count| {
            heading.child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(count.to_string()),
            )
        })
}

fn detail_settings(id: &'static str, group: SettingGroup) -> AnyElement {
    // Settings uses a virtual list, which needs a bounded viewport inside this
    // otherwise content-sized card. Keep native model/source scrolling local.
    div()
        .w_full()
        .h(px(400.))
        .child(settings_controls(
            id,
            gpui_kit::component::setting::SettingPage::new("")
                .resettable(false)
                .header_style(&StyleRefinement::default().hidden())
                .group(group),
        ))
        .into_any_element()
}

fn confirm_delete(view: Entity<ConduitApp>, id: String, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, window, _| {
        let view = view.clone();
        let id = id.clone();
        dialog
            .title(service_text("settings.models.delete_provider"))
            .w((window.viewport_size().width - px(48.)).clamp(px(0.), px(440.)))
            .content(move |body, _, _| {
                let view = view.clone();
                let id = id.clone();
                body.child(
                    v_flex()
                        .gap_4()
                        .child(service_text("settings.services.delete_confirm"))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("service-delete-cancel")
                                        .ghost()
                                        .label(service_text("git_branches.cancel"))
                                        .on_click(|_, window, cx| window.close_dialog(cx)),
                                )
                                .child(
                                    Button::new("service-delete-confirm")
                                        .primary()
                                        .label(service_text("settings.models.delete_provider"))
                                        .on_click(move |_, window, cx| {
                                            let deleted = view.update(cx, |app, cx| {
                                                app.delete_model_service(&id, cx)
                                            });
                                            window.close_dialog(cx);
                                            if !deleted {
                                                window.push_notification(
                                                    Notification::error(service_text(
                                                        "settings.services.delete_blocked",
                                                    )),
                                                    cx,
                                                );
                                            }
                                        }),
                                ),
                        ),
                )
            })
    });
}

/// Launch the installed application, without importing a route or executing a shell command.
fn launch_cc_switch() -> bool {
    use std::os::windows::process::CommandExt;
    use winreg::{
        RegKey,
        enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE},
    };
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Ok(key) =
            RegKey::predef(hive).open_subkey(r"Software\Classes\ccswitch\shell\open\command")
        else {
            continue;
        };
        let Ok(command) = key.get_value::<String, _>("") else {
            continue;
        };
        let command = command.trim();
        let executable = if let Some(quoted) = command.strip_prefix('"') {
            quoted.split('"').next()
        } else {
            command.split_whitespace().next()
        };
        let Some(path) = executable.map(PathBuf::from).filter(|path| {
            path.is_absolute()
                && path.is_file()
                && path.file_name().is_some_and(|name| {
                    name.to_string_lossy().eq_ignore_ascii_case("cc-switch.exe")
                })
        }) else {
            continue;
        };
        if std::process::Command::new(path)
            .creation_flags(0x08000000)
            .spawn()
            .is_ok()
        {
            return true;
        }
    }
    false
}

/// A shared, searchable catalog list. Render-time input reads keep it in sync with deep links.
#[derive(IntoElement)]
struct ServiceModels {
    engine: Provider,
    channel: ModelChannel,
    models: Vec<crate::models::ModelOption>,
    disabled: bool,
    defaults: ComposerSelection,
    search: Entity<InputState>,
    view: Entity<ConduitApp>,
}
impl RenderOnce for ServiceModels {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            engine,
            channel,
            models,
            disabled,
            defaults,
            search,
            view,
        } = self;
        let query = search.read(cx).value().to_lowercase();
        let mut list = v_flex().w_full().gap_0().child(
            div()
                .w_full()
                .debug_selector(|| "service-model-search".into())
                .child(
                    Input::new(&search)
                        .small()
                        .aria_label(service_text("settings.services.search_models")),
                ),
        );
        let total = models.len();
        let mut models: Vec<_> = models
            .iter()
            .filter(|model| {
                let text = format!("{} {} {}", model.id, model.label, model.note).to_lowercase();
                query.split_whitespace().all(|word| text.contains(word))
            })
            .cloned()
            .collect();
        // A model deep link uses its stable ID as the query. Put that exact row first.
        models.sort_by_key(|model| model.id.to_lowercase() != query.trim());
        let matches = models.len();
        for (index, model) in models.into_iter().enumerate() {
            let selected =
                defaults.channel == channel && defaults.model.as_ref() == Some(&model.id);
            let use_view = view.clone();
            let use_channel = channel.clone();
            let model_id = model.id.clone();
            list = list.child(
                h_flex()
                    .debug_selector(move || format!("service-model-row-{index}"))
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .flex_wrap()
                    .py_2()
                    .px_3()
                    .when(index + 1 < matches, |row| row.border_b_1())
                    .border_color(settings_group_border())
                    .when(model.id.to_lowercase() == query.trim(), |row| {
                        row.bg(rgb(palette().surface_strong))
                    })
                    .when(selected, |row| {
                        row.bg(gpui_kit::rgba((palette().activity_running << 8) | 0x08))
                            .border_color(gpui_kit::rgba((palette().activity_running << 8) | 0x55))
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(px(120.))
                            .gap_1()
                            .child(
                                div()
                                    .font_medium()
                                    .text_size(ui_text_size(13.))
                                    .child(model.label.clone()),
                            )
                            .when(model.label != model.id, |row| {
                                row.child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(rgb(palette().muted))
                                        .child(model.id),
                                )
                            })
                            .when(!model.note.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_size(ui_text_size(11.))
                                        .text_color(rgb(palette().muted))
                                        .child(model.note),
                                )
                            }),
                    )
                    .child(
                        Button::new(("service-model-default", index))
                            .debug_selector(move || format!("service-model-default-{index}"))
                            .small()
                            .ghost()
                            .label(service_text(if selected {
                                "settings.services.current_default"
                            } else {
                                "settings.services.set_new_default"
                            }))
                            .disabled(disabled || selected)
                            .tooltip(service_text("settings.services.choose_model_hint"))
                            .on_click(move |_, window, cx| {
                                use_view.update(cx, |app, cx| {
                                    app.set_service_default(
                                        engine,
                                        use_channel.clone(),
                                        Some(model_id.clone()),
                                        window,
                                        cx,
                                    );
                                })
                            }),
                    ),
            );
        }
        if total == 0 {
            list = list.child(
                div()
                    .py_4()
                    .child(service_text("settings.services.no_models")),
            );
        }
        if total > 0 && matches == 0 {
            list = list.child(div().py_3().text_color(rgb(palette().muted)).child(
                conduit::i18n::text_static(
                    "settings.model_management.no_matching_models_adjust_the_search",
                ),
            ));
        }
        list
    }
}
