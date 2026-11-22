//! Compact service management; routes and defaults share the existing registry.
use super::*;
use gpui_kit::component::notification::Notification;
mod details;
mod list;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DetailTab {
    Models,
    Sessions,
    Sources,
}

fn service_text(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

fn service_count(count: usize) -> String {
    conduit::i18n::format("settings.services.count", &[("count", count.to_string())])
}

impl ConduitApp {
    /// Reject stale controls, read-only windows and mismatched engine routes.
    pub(super) fn set_service_default(
        &mut self,
        engine: Provider,
        channel: ModelChannel,
        model: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.writable || self.closing {
            return false;
        }
        match &channel {
            ModelChannel::Cli => {}
            ModelChannel::Provider(id) => {
                if !engine.descriptor().capabilities.model_services
                    || !self.store.providers.iter().any(|provider| {
                        provider.id == *id
                            && provider.engine == engine
                            && model
                                .as_ref()
                                .is_none_or(|id| provider.catalog().iter().any(|m| m.id == *id))
                    })
                {
                    return false;
                }
            }
            ModelChannel::Ccs(_) => return false,
        }
        self.store
            .session_defaults
            .insert(engine, crate::store::SessionDefaults { channel, model });
        self.persist();
        cx.notify();
        true
    }

    pub(super) fn delete_model_service(&mut self, id: &str, cx: &mut Context<Self>) -> bool {
        if !self.writable
            || self.closing
            || !self.provider_references(id).is_empty()
            || !self
                .store
                .providers
                .iter()
                .any(|p| p.id == id && p.source == ProviderSource::Local)
        {
            return false;
        }
        self.store.providers.retain(|p| p.id != id);
        self.persist();
        cx.emit(crate::providers::ProviderCatalogChanged);
        cx.notify();
        true
    }

    fn service_session_count(&self, engine: Provider, channel: &ModelChannel) -> usize {
        self.store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .filter(|s| s.provider == engine && &s.channel == channel)
            .count()
    }

    pub(super) fn model_services_page(
        &self,
        width: f32,
        actions: AnyElement,
        view: Entity<Self>,
    ) -> AnyElement {
        let engine = self.connection_settings.engine;
        let supports_services = engine.descriptor().capabilities.model_services;
        let sources = view.clone();
        let native = view.clone();
        let inset = settings_content_inset();
        let split =
            width - inset * 2. >= 720. * (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE);
        let selected = self.selected_service_channel(engine);
        let detail_only = !split && self.connection_settings.service_detail_open;
        let count = self
            .store
            .providers
            .iter()
            .filter(|p| p.engine == engine)
            .count();
        let body = v_flex()
            .id("model-services-scroll")
            .test_support()
            .debug_selector(|| "model-services-scroll".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(if detail_only {
                &self.connection_settings.service_detail_scroll
            } else {
                &self.connection_settings.service_scroll
            })
            .px(px(inset))
            .pb_3()
            .gap_3()
            .when(!detail_only, |body| {
                body.child(self.service_default_summary(engine, view.clone()))
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .flex_wrap()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        div().font_semibold().text_size(ui_text_size(14.)).child(
                                            service_text("settings.defaults.model_services"),
                                        ),
                                    )
                                    .when(supports_services, |row| {
                                        row.child(
                                            div()
                                                .text_color(rgb(palette().muted))
                                                .text_size(ui_text_size(12.))
                                                .child(service_count(count)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(service_text("settings.services.new_sessions_only")),
                            ),
                    )
                    .child(actions),
            )
            .when(supports_services, |body| {
                body.child(
                    h_flex()
                        .id("service-workbench")
                        .debug_selector(|| "service-workbench".into())
                        .w_full()
                        .items_start()
                        .gap_3()
                        .when(!detail_only, |row| {
                            row.child(
                                div()
                                    .min_w_0()
                                    .when(split, |list| {
                                        list.w(px(280.
                                            * (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE)))
                                    })
                                    .when(!split, |list| list.w_full())
                                    .flex_shrink_0()
                                    .child(self.service_list(engine, &selected, view.clone())),
                            )
                        })
                        .when(split || detail_only, |row| {
                            let back = view.clone();
                            row.child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_3()
                                    .when(detail_only, |detail| {
                                        detail.child(
                                            Button::new("service-detail-back")
                                                .debug_selector(|| "service-detail-back".into())
                                                .ghost()
                                                .small()
                                                .icon(FullIconName::ArrowLeft)
                                                .label(service_text(
                                                    "settings.services.back_to_list",
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    back.update(cx, |app, cx| {
                                                        app.connection_settings
                                                            .service_detail_open = false;
                                                        cx.notify();
                                                    })
                                                }),
                                        )
                                    })
                                    .child(self.service_details(
                                        engine,
                                        &selected,
                                        self.connection_settings.service_tab,
                                        view.clone(),
                                    )),
                            )
                        }),
                )
            })
            .when(!supports_services, |body| {
                body.child(
                    div()
                        .p_4()
                        .rounded_lg()
                        .border_1()
                        .border_color(rgb(palette().border))
                        .text_size(ui_text_size(13.))
                        .text_color(rgb(palette().muted))
                        .child(service_text("settings.services.native_only")),
                )
            })
            .when(!supports_services, |body| {
                body.child(self.service_details(
                    engine,
                    &ModelChannel::Cli,
                    DetailTab::Models,
                    view.clone(),
                ))
            })
            .when(supports_services && !detail_only, |body| {
                body.child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            Button::new("services-native-catalog")
                                .small()
                                .ghost()
                                .label(service_text("settings.services.native_catalog"))
                                .on_click(move |_, window, cx| {
                                    native.update(cx, |app, cx| {
                                        app.open_service_details(
                                            engine,
                                            ModelChannel::Cli,
                                            DetailTab::Models,
                                            window,
                                            cx,
                                        )
                                    })
                                }),
                        )
                        .child(
                            Button::new("services-config-sources")
                                .small()
                                .ghost()
                                .label(service_text("settings.models.view_configuration_sources"))
                                .on_click(move |_, window, cx| {
                                    sources.update(cx, |app, cx| {
                                        app.open_service_details(
                                            engine,
                                            ModelChannel::Cli,
                                            DetailTab::Sources,
                                            window,
                                            cx,
                                        )
                                    })
                                }),
                        ),
                )
            })
            .when_some(
                self.engine_catalog(engine).and_then(|c| c.error.clone()),
                |body, error| {
                    body.child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().error_text))
                            .child(error),
                    )
                },
            )
            .when_some(
                supports_services.then(|| self.ccs.error.clone()).flatten(),
                |body, error| {
                    body.child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().error_text))
                            .child(error),
                    )
                },
            );
        v_flex()
            .size_full()
            .min_w_0()
            .child(super::management::management_toolbar_inset(
                engine,
                false,
                None,
                width,
                self.store.ui.ui_font_size,
                view,
                inset,
            ))
            .child(body)
            .into_any_element()
    }

    fn service_default_summary(&self, engine: Provider, view: Entity<Self>) -> AnyElement {
        let selection = self.default_selection(engine);
        let channel = selection.channel.clone();
        let label = if channel == ModelChannel::Cli {
            service_text("settings.defaults.native_cli_configuration").to_string()
        } else {
            self.channel_label(engine, &channel)
        };
        let model = selection
            .model
            .clone()
            .or_else(|| {
                (channel == ModelChannel::Cli)
                    .then(|| self.engine_catalog(engine).and_then(|c| c.current.clone()))
                    .flatten()
            })
            .unwrap_or_else(|| service_text("settings.defaults.use_service_default_model").into());
        h_flex()
            .debug_selector(|| "service-default-summary".into())
            .w_full()
            .flex_shrink_0()
            .flex_wrap()
            .gap_3()
            .justify_between()
            .px_3()
            .py_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(160.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::format(
                                "settings.services.default_for_engine",
                                &[("engine", engine.label().into())],
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                div()
                                    .debug_selector(|| "services-default-name".into())
                                    .font_semibold()
                                    .text_size(ui_text_size(14.))
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(model),
                            ),
                    ),
            )
            .child(
                Button::new("services-default-catalog")
                    .debug_selector(|| "services-default-catalog".into())
                    .small()
                    .outline()
                    .label(service_text("settings.models.manage_model_catalog"))
                    .icon(FullIconName::ChevronRight)
                    .on_click(move |_, window, cx| {
                        view.update(cx, |app, cx| {
                            app.open_service_details(
                                engine,
                                channel.clone(),
                                DetailTab::Models,
                                window,
                                cx,
                            )
                        })
                    }),
            )
            .into_any_element()
    }

    // Keep the existing group indices for settings search results.
    pub(super) fn providers_group(
        &self,
        engine: Provider,
        _disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let keywords: Vec<SharedString> = self
            .store
            .providers
            .iter()
            .filter(|p| p.engine == engine)
            .map(|p| p.name.clone().into())
            .collect();
        SettingGroup::new().variant(GroupBoxVariant::Normal).item(
            SettingItem::render(move |_, window, cx| {
                let width = f32::from(window.viewport_size().width) - 260.;
                let app = view.read(cx);
                v_flex()
                    .gap_4()
                    .w_full()
                    .child(app.service_default_summary(engine, view.clone()))
                    .child(app.service_table(engine, width, view.clone()))
            })
            .keywords(keywords),
        )
    }

    fn service_table(&self, engine: Provider, width: f32, view: Entity<Self>) -> AnyElement {
        let compact = width < 780. * (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE);
        let disabled = !self.writable || self.closing;
        let defaults = self.default_selection(engine);
        let providers: Vec<_> = self
            .store
            .providers
            .iter()
            .filter(|p| p.engine == engine)
            .collect();
        let mut table = v_flex()
            .w_full()
            .flex_shrink_0()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface));
        if providers.is_empty() {
            return table
                .child(
                    div()
                        .p_6()
                        .text_size(ui_text_size(13.))
                        .text_color(rgb(palette().muted))
                        .child(service_text(
                            "settings.models.no_model_services_yet_add_endpoints_and",
                        )),
                )
                .into_any_element();
        }
        if !compact {
            table = table.child(
                h_flex()
                    .w_full()
                    .px_4()
                    .py_2()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(
                        div()
                            .flex_1()
                            .child(service_text("settings.services.service")),
                    )
                    .child(
                        div()
                            .w(px(96.))
                            .child(service_text("settings.services.source")),
                    )
                    .child(
                        div()
                            .w(px(72.))
                            .child(service_text("settings.models.model_list")),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .child(service_text("settings.services.sessions")),
                    )
                    .child(
                        div()
                            .w(px(140.))
                            .text_right()
                            .child(service_text("settings.services.actions")),
                    ),
            );
        }
        for (index, provider) in providers.iter().enumerate() {
            let channel = ModelChannel::Provider(provider.id.clone());
            let current = defaults.channel == channel;
            let imported = matches!(provider.source, ProviderSource::Ccs { .. });
            let sessions = self.service_session_count(engine, &channel);
            let models_view = view.clone();
            let models_channel = channel.clone();
            let sessions_view = view.clone();
            let sessions_channel = channel.clone();
            let default_view = view.clone();
            let default_channel = channel.clone();
            let detail_view = view.clone();
            let detail_channel = channel;
            let model_button = Button::new(("service-models", index))
                .debug_selector(move || format!("service-models-{index}"))
                .small()
                .ghost()
                .label(if compact {
                    conduit::i18n::format(
                        "settings.services.models_count",
                        &[("count", provider.catalog().len().to_string())],
                    )
                } else {
                    service_count(provider.catalog().len())
                })
                .icon(FullIconName::ChevronRight)
                .accessibility_label(format!(
                    "{} · {}",
                    provider.name,
                    service_text("settings.models.model_list")
                ))
                .on_click(move |_, window, cx| {
                    models_view.update(cx, |app, cx| {
                        app.open_service_details(
                            engine,
                            models_channel.clone(),
                            DetailTab::Models,
                            window,
                            cx,
                        )
                    })
                });
            let session_button = Button::new(("service-sessions", index))
                .debug_selector(move || format!("service-sessions-{index}"))
                .small()
                .ghost()
                .label(if compact {
                    conduit::i18n::format(
                        "settings.services.sessions_count",
                        &[("count", sessions.to_string())],
                    )
                } else {
                    service_count(sessions)
                })
                .icon(FullIconName::ChevronRight)
                .accessibility_label(format!(
                    "{} · {}",
                    provider.name,
                    service_text("settings.services.sessions")
                ))
                .on_click(move |_, window, cx| {
                    sessions_view.update(cx, |app, cx| {
                        app.open_service_details(
                            engine,
                            sessions_channel.clone(),
                            DetailTab::Sessions,
                            window,
                            cx,
                        )
                    })
                });
            let description = v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .debug_selector(move || format!("service-description-{index}"))
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_semibold()
                        .child(provider.name.clone()),
                )
                .child(
                    div()
                        .id(("service-endpoint", index))
                        .truncate()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .tooltip({
                            let endpoint = provider.base_url.clone();
                            move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(endpoint.clone())
                                    .build(window, cx)
                            }
                        })
                        .child(provider.base_url.clone()),
                );
            let source = if imported {
                "cc-switch"
            } else {
                service_text("settings.services.local")
            };
            let controls = h_flex()
                .gap_1()
                .flex_shrink_0()
                .justify_end()
                .debug_selector(move || format!("service-actions-{index}"))
                .when(!compact, |row| row.w(px(140.)))
                .child(
                    Button::new(("provider-use-default", index))
                        .debug_selector(move || format!("service-default-{index}"))
                        .small()
                        .outline()
                        .label(service_text(if current {
                            "settings.services.current_default"
                        } else {
                            "settings.services.set_default"
                        }))
                        .disabled(disabled || current)
                        .tooltip(service_text("settings.services.new_sessions_only"))
                        .on_click(move |_, window, cx| {
                            default_view.update(cx, |app, cx| {
                                if app.set_service_default(
                                    engine,
                                    default_channel.clone(),
                                    None,
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
                )
                .child(
                    Button::new(("service-details", index))
                        .small()
                        .ghost()
                        .icon(FullIconName::Ellipsis)
                        .accessibility_label(service_text("settings.services.details"))
                        .tooltip(service_text("settings.services.details"))
                        .on_click(move |_, window, cx| {
                            detail_view.update(cx, |app, cx| {
                                app.open_service_details(
                                    engine,
                                    detail_channel.clone(),
                                    DetailTab::Models,
                                    window,
                                    cx,
                                )
                            })
                        }),
                );
            let row = if compact {
                v_flex()
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py_3()
                    .child(
                        h_flex().w_full().gap_2().child(description).child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(source),
                        ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .gap_2()
                            .flex_wrap()
                            .child(h_flex().gap_1().child(model_button).child(session_button))
                            .child(controls),
                    )
                    .into_any_element()
            } else {
                h_flex()
                    .w_full()
                    .gap_3()
                    .px_4()
                    .py_3()
                    .child(description)
                    .child(
                        div()
                            .w(px(96.))
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(source),
                    )
                    .child(div().w(px(72.)).child(model_button))
                    .child(div().w(px(80.)).child(session_button))
                    .child(controls)
                    .into_any_element()
            };
            table = table.child(
                div()
                    .w_full()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(rgb(palette().border))
                    })
                    .when(current, |row| {
                        row.bg(gpui_kit::rgba((palette().activity_running << 8) | 0x0a))
                    })
                    .child(row),
            );
        }
        table.into_any_element()
    }

    fn open_service_details(
        &mut self,
        engine: Provider,
        channel: ModelChannel,
        tab: DetailTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.connection_settings.models_expanded = true;
        self.connection_settings.sources_expanded = true;
        self.connection_settings.engine = engine;
        if self.connection_settings.service_selection.as_ref() != Some(&(engine, channel.clone())) {
            self.connection_settings
                .service_detail_scroll
                .set_offset(point(px(0.), px(0.)));
        }
        self.connection_settings.service_selection = Some((engine, channel));
        self.connection_settings.service_detail_open = true;
        self.connection_settings.service_tab = tab;
        self.settings_ui.group = None;
        if self.settings_ui.page != 8 {
            self.select_settings_page(8, window, cx);
        }
        cx.notify();
    }
}
