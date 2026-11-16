use super::*;

pub(super) struct ServiceDetails {
    app: WeakEntity<ConduitApp>,
    engine: Provider,
    channel: ModelChannel,
    tab: DetailTab,
    _subscription: Subscription,
}

impl ServiceDetails {
    pub(super) fn new(
        app: Entity<ConduitApp>,
        engine: Provider,
        channel: ModelChannel,
        tab: DetailTab,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&app, |_, _, cx| cx.notify());
        Self {
            app: app.downgrade(),
            engine,
            channel,
            tab,
            _subscription: subscription,
        }
    }
}

impl Render for ServiceDetails {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(view) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = view.read(cx);
        let engine = self.engine;
        let channel = self.channel.clone();
        let provider = match &channel {
            ModelChannel::Provider(id) => app
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine)
                .cloned(),
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
        let native = channel == ModelChannel::Cli;
        let imported = provider
            .as_ref()
            .is_some_and(|p| matches!(p.source, ProviderSource::Ccs { .. }));
        let disabled = !app.writable || app.closing;
        let label = if native {
            service_text("settings.defaults.native_cli_configuration").to_string()
        } else {
            app.channel_label(engine, &channel)
        };
        let models = if native {
            app.managed_native_models(engine, true)
        } else {
            provider.as_ref().map(|p| p.catalog()).unwrap_or_default()
        };
        let session_count = app.service_session_count(engine, &channel);
        let mut tabs = h_flex()
            .w_full()
            .gap_2()
            .pb_2()
            .border_b_1()
            .border_color(rgb(palette().border));
        for (index, (tab, title)) in [
            (
                DetailTab::Models,
                format!(
                    "{} · {}",
                    service_text("settings.models.model_list"),
                    models.len()
                ),
            ),
            (
                DetailTab::Sessions,
                format!(
                    "{} · {}",
                    service_text("settings.services.sessions"),
                    session_count
                ),
            ),
            (
                DetailTab::Sources,
                service_text("settings.models.configuration_sources").to_string(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            if tab == DetailTab::Sources
                && (!native || !engine.descriptor().capabilities.model_services)
            {
                continue;
            }
            tabs = tabs.child(
                Button::new(("service-detail-tab", index))
                    .debug_selector(move || format!("service-detail-tab-{index}"))
                    .small()
                    .ghost()
                    .selected(self.tab == tab)
                    .label(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    })),
            );
        }
        let mut header = v_flex()
            .w_full()
            .gap_2()
            .pb_3()
            .child(
                div()
                    .text_size(ui_text_size(18.))
                    .font_semibold()
                    .child(label),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(if imported {
                        "cc-switch"
                    } else if native {
                        engine.label()
                    } else {
                        service_text("settings.services.local")
                    }),
            );
        if let Some(provider) = &provider {
            header = header.child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(provider.base_url.clone()),
            );
        }
        let body = match self.tab {
            DetailTab::Sources => {
                let group = app.models_source_group(engine, view.clone());
                detail_settings("service-source-settings", group)
            }
            DetailTab::Models if native => detail_settings(
                "service-native-model-settings",
                app.models_provider_group(engine, view.clone()),
            ),
            DetailTab::Models => {
                let defaults = app.default_selection(engine);
                let mut list = v_flex().w_full().gap_2().child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(service_text("settings.services.choose_model_hint")),
                );
                for (index, model) in models.into_iter().enumerate() {
                    let selected =
                        defaults.channel == channel && defaults.model.as_ref() == Some(&model.id);
                    let use_view = view.clone();
                    let use_channel = channel.clone();
                    let model_id = model.id.clone();
                    list = list.child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .flex_wrap()
                            .py_3()
                            .border_b_1()
                            .border_color(rgb(palette().border))
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
                                    .small()
                                    .ghost()
                                    .label(service_text(if selected {
                                        "settings.services.current_default"
                                    } else {
                                        "settings.services.set_default"
                                    }))
                                    .disabled(disabled || selected)
                                    .on_click(move |_, window, cx| {
                                        use_view.update(cx, |app, cx| {
                                            if app.set_service_default(
                                                engine,
                                                use_channel.clone(),
                                                Some(model_id.clone()),
                                                cx,
                                            ) {
                                                window.push_notification(
                                                    Notification::success(service_text(
                                                        "settings.services.default_saved",
                                                    )),
                                                    cx,
                                                );
                                            }
                                        })
                                    }),
                            ),
                    );
                }
                if provider.as_ref().is_none_or(|p| p.catalog().is_empty()) {
                    list = list.child(
                        div()
                            .py_4()
                            .child(service_text("settings.services.no_models")),
                    );
                }
                list.into_any_element()
            }
            DetailTab::Sessions => {
                let mut list = v_flex().w_full().gap_2().child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(service_text("settings.services.new_sessions_only")),
                );
                if session_count == 0 {
                    list = list.child(
                        div()
                            .py_4()
                            .text_size(ui_text_size(13.))
                            .child(service_text("settings.services.no_sessions")),
                    );
                }
                for workspace in &app.store.workspaces {
                    for session in workspace
                        .sessions
                        .iter()
                        .filter(|s| s.provider == engine && s.channel == channel)
                    {
                        list = list.child(
                            v_flex()
                                .w_full()
                                .py_3()
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
                    let uses = app.provider_configuration_references(&provider.id);
                    if !uses.is_empty() {
                        list = list
                            .child(
                                div()
                                    .pt_4()
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
        };
        v_flex()
            .id("service-details")
            .test_support()
            .debug_selector(|| "service-details".into())
            .size_full()
            .min_h_0()
            .gap_3()
            .px_3()
            .pb_3()
            .child(header)
            .child(tabs)
            .child(
                div()
                    .id("service-detail-body")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }
}

impl ServiceDetails {
    pub(super) fn footer(&self, cx: &App) -> AnyElement {
        let Some(view) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let app = view.read(cx);
        let engine = self.engine;
        let channel = self.channel.clone();
        let provider = match &channel {
            ModelChannel::Provider(id) => app
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
        let disabled = !app.writable || app.closing;
        let defaults = app.default_selection(engine);
        let current = defaults.channel == channel && defaults.model.is_none();
        let use_view = view.clone();
        let use_channel = channel;
        let mut footer = h_flex()
            .w_full()
            .justify_between()
            .gap_2()
            .flex_wrap()
            .pt_3()
            .border_t_1()
            .border_color(rgb(palette().border));
        if let Some(provider) = provider {
            if imported {
                footer = footer.child(
                    Button::new("service-edit-cc-switch")
                        .small()
                        .outline()
                        .icon(FullIconName::ExternalLink)
                        .label(service_text("settings.services.open_cc_switch"))
                        .disabled(app.closing)
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
                let in_use = !app.provider_references(&provider.id).is_empty();
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
        footer = footer.child(
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
                .on_click(move |_, window, cx| {
                    use_view.update(cx, |app, cx| {
                        if app.set_service_default(engine, use_channel.clone(), None, cx) {
                            window.push_notification(
                                Notification::success(service_text(
                                    "settings.services.default_saved",
                                )),
                                cx,
                            );
                        }
                    })
                }),
        );

        v_flex()
            .w_full()
            .gap_2()
            .when(imported, |body| {
                body.child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(service_text("settings.services.cc_switch_hint")),
                )
            })
            .child(footer)
            .into_any_element()
    }
}

fn detail_settings(id: &'static str, group: SettingGroup) -> AnyElement {
    Settings::new(id)
        .sidebar_width(px(0.))
        .sidebar_size_range(px(0.)..px(0.))
        .sidebar_style(&StyleRefinement::default().hidden())
        .page(
            gpui_kit::component::setting::SettingPage::new("")
                .resettable(false)
                .header_style(&StyleRefinement::default().hidden())
                .group(group),
        )
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
                                            if deleted {
                                                window.close_sheet(cx);
                                            } else {
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
