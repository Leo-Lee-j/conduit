use super::*;

pub(super) fn service_badge(label: &'static str) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .px_2()
        .py_0p5()
        .rounded_md()
        .text_size(ui_text_size(10.))
        .font_medium()
        .text_color(rgb(palette().activity_running))
        .bg(gpui_kit::rgba((palette().activity_running << 8) | 0x16))
        .child(label)
}

pub(super) fn service_avatar(name: &str, selected: bool) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .font_semibold()
        .text_size(ui_text_size(13.))
        .bg(rgb(palette().surface_strong))
        .when(selected, |avatar| {
            avatar
                .text_color(rgb(palette().activity_running))
                .bg(gpui_kit::rgba((palette().activity_running << 8) | 0x18))
        })
        .child(name.chars().next().unwrap_or('?').to_string())
}

impl ConduitApp {
    pub(super) fn selected_service_channel(&self, engine: Provider) -> ModelChannel {
        let valid = |channel: &ModelChannel| match channel {
            ModelChannel::Cli => true,
            ModelChannel::Ccs(_) => false,
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .any(|p| p.engine == engine && p.id == *id),
        };
        if let Some((selected_engine, channel)) = &self.connection_settings.service_selection
            && *selected_engine == engine
            && valid(channel)
        {
            return channel.clone();
        }
        let defaults = self.default_selection(engine);
        if matches!(defaults.channel, ModelChannel::Provider(_)) && valid(&defaults.channel) {
            return defaults.channel;
        }
        self.store
            .providers
            .iter()
            .find(|p| p.engine == engine)
            .map(|p| ModelChannel::Provider(p.id.clone()))
            .unwrap_or(ModelChannel::Cli)
    }

    pub(super) fn service_list(
        &self,
        engine: Provider,
        selected: &ModelChannel,
        view: Entity<Self>,
    ) -> AnyElement {
        let defaults = self.default_selection(engine);
        let providers: Vec<_> = self
            .store
            .providers
            .iter()
            .filter(|p| p.engine == engine)
            .enumerate()
            .collect();
        let mut list = v_flex()
            .id("service-list")
            .test_support()
            .debug_selector(|| "service-list".into())
            .w_full()
            .min_w_0()
            .p_2()
            .gap_0p5()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface));
        for imported in [true, false] {
            let group: Vec<_> = providers
                .iter()
                .filter(|(_, p)| matches!(p.source, ProviderSource::Ccs { .. }) == imported)
                .collect();
            if group.is_empty() {
                continue;
            }
            list =
                list.child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .px_1()
                        .pt_2()
                        .pb_1()
                        .child(div().text_size(ui_text_size(11.)).font_medium().child(
                            if imported {
                                "cc-switch"
                            } else {
                                service_text("settings.services.local")
                            },
                        ))
                        .child(
                            div()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(group.len().to_string()),
                        ),
                );
            for (index, provider) in group {
                let index = *index;
                let channel = ModelChannel::Provider(provider.id.clone());
                let active = selected == &channel;
                let current = defaults.channel == channel;
                let open = view.clone();
                let sessions = self.service_session_count(engine, &channel);
                let count = provider.catalog().len();
                let endpoint = provider
                    .base_url
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .trim_end_matches('/')
                    .to_string();
                list = list.child(
                    Button::new(SharedString::from(format!("service-row-{}", provider.id)))
                        .debug_selector(move || format!("service-row-{index}"))
                        .accessibility_label(provider.name.clone())
                        .tooltip(format!(
                            "{} · {}",
                            provider.base_url,
                            conduit::i18n::format(
                                "settings.services.sessions_count",
                                &[("count", sessions.to_string())]
                            )
                        ))
                        .ghost()
                        .selected(active)
                        .w_full()
                        .min_w_0()
                        .h_auto()
                        .px_1()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(gpui_kit::rgba(0))
                        .when(active, |row| {
                            row.bg(gpui_kit::rgba((palette().activity_running << 8) | 0x0c))
                                .border_color(gpui_kit::rgba(
                                    (palette().activity_running << 8) | 0x55,
                                ))
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .child(service_avatar(&provider.name, active))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .min_w_0()
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(ui_text_size(12.))
                                                        .font_semibold()
                                                        .child(provider.name.clone()),
                                                )
                                                .when(current, |row| {
                                                    row.child(service_badge(service_text(
                                                        "settings.services.current_default",
                                                    )))
                                                }),
                                        )
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .min_w_0()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(ui_text_size(10.))
                                                        .text_color(rgb(palette().muted))
                                                        .child(endpoint),
                                                )
                                                .child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .text_size(ui_text_size(10.))
                                                        .text_color(rgb(palette().muted))
                                                        .child(conduit::i18n::format(
                                                            "settings.services.models_count",
                                                            &[("count", count.to_string())],
                                                        )),
                                                ),
                                        ),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            open.update(cx, |app, cx| {
                                app.open_service_details(
                                    engine,
                                    channel.clone(),
                                    DetailTab::Models,
                                    window,
                                    cx,
                                );
                            })
                        }),
                );
            }
        }
        if providers.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(service_text(
                        "settings.models.no_model_services_yet_add_endpoints_and",
                    )),
            );
        }
        let native = view;
        list.child(
            Button::new("service-native-row")
                .accessibility_label(service_text("settings.defaults.native_cli_configuration"))
                .debug_selector(|| "service-native-row".into())
                .ghost()
                .w_full()
                .h_auto()
                .px_1()
                .py_2()
                .selected(*selected == ModelChannel::Cli)
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .text_size(ui_text_size(11.))
                                .child(service_text("settings.defaults.native_cli_configuration")),
                        )
                        .when(defaults.channel == ModelChannel::Cli, |row| {
                            row.child(service_badge(service_text(
                                "settings.services.current_default",
                            )))
                        }),
                )
                .on_click(move |_, window, cx| {
                    native.update(cx, |app, cx| {
                        app.open_service_details(
                            engine,
                            ModelChannel::Cli,
                            DetailTab::Models,
                            window,
                            cx,
                        );
                    })
                }),
        )
        .into_any_element()
    }
}
