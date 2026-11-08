//! Provider, permission, and model selection controls.

use super::generation::GenerationKind;
use super::*;

// Keep notifications local. Observing the parent ConduitApp here creates a
// parent/child invalidation cycle while the popover is open.
struct SessionModelPicker {
    app: Entity<ConduitApp>,
    session_id: u64,
    search: Entity<InputState>,
    slider: Entity<SliderState>,
    _subscriptions: Vec<Subscription>,
}

impl SessionModelPicker {
    fn new(
        app: Entity<ConduitApp>,
        session_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let effort = app.read(cx).composer_selection(session_id).effort;
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(4.)
                .step(1.)
                .default_value(effort.index() as f32)
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(conduit::i18n::text_static(
                "model_picker.search_model_names_or_ids",
            ))
        });
        let view = cx.entity();
        let subscriptions = vec![
            cx.subscribe(
                &app,
                |_, _, _: &crate::providers::ProviderCatalogChanged, cx| cx.notify(),
            ),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
            window.subscribe(&slider, cx, move |_, event: &SliderEvent, window, cx| {
                if let SliderEvent::Change(value) = event {
                    let effort = ReasoningEffort::ALL[(value.start().round() as usize).min(4)];
                    view.update(cx, |this, cx| {
                        // SliderState snaps the emitted value to the configured
                        // step, but keeps the pointer percentage while dragging.
                        // Write the snapped value back so the thumb also moves in
                        // the five discrete reasoning levels.
                        this.slider.update(cx, |slider, cx| {
                            slider.set_value(effort.index() as f32, window, cx)
                        });
                        this.app.update(cx, |app, cx| {
                            app.update_composer_selection(this.session_id, cx, |s| {
                                s.effort = effort
                            });
                        });
                        cx.notify();
                    });
                }
            }),
        ];
        Self {
            app,
            session_id,
            search,
            slider,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for SessionModelPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.app.read(cx);
        let selection = app.composer_selection(self.session_id);
        let engine = selection.engine;
        let channel = selection.channel;
        let selected = selection.model;
        let effort = selection.effort;
        let native_models = if engine == Provider::OpenCode {
            let catalog_models = app.channel_models(engine, &ModelChannel::Cli).1;
            if !catalog_models.is_empty() {
                catalog_models
            } else {
                app.native_model_preferences(engine)
                    .merge(&selection.opencode.models(), false)
            }
        } else {
            vec![]
        };
        let (note, mut models) = if engine == Provider::OpenCode && channel == ModelChannel::Cli {
            (
                app.channel_models(engine, &ModelChannel::Cli).0,
                native_models.clone(),
            )
        } else {
            app.channel_models(engine, &channel)
        };
        let variants = selection.opencode.variants();
        let variant = selection.opencode.effective_variant().map(str::to_owned);
        // Native OpenCode providers are encoded in provider/model IDs. Keep
        // the CLI route and filter within its actual catalog, never inventing
        // an API endpoint or copying credentials into a custom provider.
        let native_provider = (engine == Provider::OpenCode && channel == ModelChannel::Cli)
            .then(|| {
                selected
                    .as_deref()
                    .filter(|id| native_models.iter().any(|model| model.id == *id))?
                    .split_once('/')
                    .map(|(p, _)| p.to_string())
            })
            .flatten();
        let native_providers: BTreeMap<String, String> = if engine == Provider::OpenCode {
            let mut providers = BTreeMap::new();
            for model in &native_models {
                if let Some((provider, _)) = model.id.split_once('/') {
                    providers
                        .entry(provider.to_string())
                        .or_insert_with(|| model.id.clone());
                }
            }
            providers
        } else {
            BTreeMap::new()
        };
        let channel_label = native_provider
            .clone()
            .unwrap_or_else(|| app.channel_label(engine, &channel));
        let pending = app.model_picker_session_id(self.session_id).is_none();
        let disabled = !app.writable
            || app.closing
            || app
                .model_picker_session_id(self.session_id)
                .is_some_and(|id| app.session_busy(id));
        let mut channels = vec![(
            ModelChannel::Cli,
            None,
            if engine == Provider::OpenCode {
                conduit::i18n::text_static("model_picker.opencode_native")
            } else {
                conduit::i18n::text_static("generation.cli_default")
            }
            .to_string(),
        )];
        channels.extend(
            native_providers
                .into_iter()
                .map(|(name, model)| (ModelChannel::Cli, Some(model), name)),
        );
        if engine != Provider::OpenCode {
            channels.extend(
                app.store
                    .providers
                    .iter()
                    .filter(|p| p.engine == engine)
                    .map(|p| (ModelChannel::Provider(p.id.clone()), None, p.name.clone())),
            );
        }
        if let Some(id) = &selected
            && !models.iter().any(|m| m.id == *id)
        {
            models::push_model(
                &mut models,
                id,
                id,
                conduit::i18n::text_static("model_picker.current_session_model"),
            );
        }
        if let Some(provider) = &native_provider {
            models.retain(|model| model.id.split_once('/').is_some_and(|(p, _)| p == provider));
        }
        let query = self.search.read(cx).value().trim().to_lowercase();
        models.retain(|m| {
            query.is_empty()
                || m.id.to_lowercase().contains(&query)
                || m.label.to_lowercase().contains(&query)
        });
        let count = models.len();
        let show_default = native_provider.is_none();
        let picker = cx.entity();
        let channel_picker = Popover::new("session-channel-picker")
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new("session-channel-trigger")
                    .ghost()
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .disabled(disabled)
                    .accessibility_label(conduit::i18n::text_static("model_picker.select_provider"))
                    .cursor_pointer()
                    .child(
                        h_flex()
                            .debug_selector(|| "session-channel-control".into())
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().faint))
                                    .child(conduit::i18n::text_static("common.provider")),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().text))
                                    .child(channel_label),
                            )
                            .child(Icon::new(IconName::ChevronDown).small()),
                    ),
            )
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("session-channel-list")
                    .w(px(300.))
                    .max_h(px(280.))
                    .overflow_y_scroll()
                    .gap_1();
                for (index, (option, native_model, label)) in channels.iter().enumerate() {
                    let active = *option == channel
                        && match native_model {
                            Some(model) => {
                                native_provider.as_deref() == model.split_once('/').map(|(p, _)| p)
                            }
                            None => native_provider.is_none(),
                        };
                    let option = option.clone();
                    let native_model = native_model.clone();
                    let picker = picker.clone();
                    let popover = popover.clone();
                    list = list.child(
                        Button::new(("session-channel", index))
                            .ghost()
                            .w_full()
                            .h(px(34.))
                            .px_2()
                            .disabled(disabled)
                            .accessibility_label(format!("Provider {label}"))
                            .cursor_pointer()
                            .when(active, |b| b.bg(rgb(palette().surface_hover)))
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("session-provider-option-{index}")
                                            })
                                            .flex_1()
                                            .truncate()
                                            .child(label.clone()),
                                    )
                                    .when(active, |r| r.child(Icon::new(IconName::Check).small())),
                            )
                            .on_click(move |_, window, cx| {
                                picker.update(cx, |this, cx| {
                                    this.app.update(cx, |app, cx| {
                                        app.update_composer_selection(this.session_id, cx, |s| {
                                            if active {
                                                return;
                                            }
                                            if s.channel != option {
                                                s.opencode = Default::default();
                                            }
                                            s.channel = option.clone();
                                            s.set_model(native_model.clone());
                                            s.effort = ReasoningEffort::for_provider(s.engine);
                                        });
                                    });
                                    this.search.update(cx, |s, cx| s.set_value("", window, cx));
                                    let effort = this
                                        .app
                                        .read(cx)
                                        .composer_selection(this.session_id)
                                        .effort;
                                    this.slider.update(cx, |s, cx| {
                                        s.set_value(effort.index() as f32, window, cx)
                                    });
                                    cx.notify();
                                });
                                if let Some(p) = popover.upgrade() {
                                    p.update(cx, |p, cx| p.dismiss(window, cx));
                                }
                            }),
                    );
                }
                list
            });
        let mut list = v_flex()
            .id("session-model-options")
            .debug_selector(|| "session-model-list".into())
            // Leave space for the four controls, popover padding and composer
            // at the minimum 850x500 window size; only the model list scrolls.
            // The composer area's outer spacing (12 above, 16 below) is part
            // of the budget, so this grows with it.
            .max_h((window.viewport_size().height - px(469.)).clamp(px(34.), px(250.)))
            .overflow_y_scroll()
            .gap_1();
        let options = std::iter::once((
            None,
            conduit::i18n::text_static("model_picker.default_model").to_string(),
        ))
        .filter(|_| show_default)
        .chain(
            models
                .into_iter()
                .map(|m| (Some(m.id.clone()), m.label.clone())),
        );
        for (index, (model, label)) in options.enumerate() {
            let active = selected == model;
            let mark = model_mark(engine, model.as_deref());
            list = list.child(
                Button::new(("session-model-option", index))
                    .ghost()
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .rounded(px(7.))
                    .disabled(disabled)
                    .cursor_pointer()
                    .accessibility_label(conduit::i18n::format(
                        "model_picker.model",
                        &[("label", label.to_string())],
                    ))
                    .when(active, |b| b.bg(rgb(palette().surface_hover)))
                    .child(
                        h_flex()
                            .debug_selector(move || format!("session-model-option-{index}"))
                            .w_full()
                            .items_center()
                            .gap_2()
                            .child(mark)
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().text))
                                    .child(label),
                            )
                            .when(active, |r| r.child(Icon::new(IconName::Check).small())),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.app.update(cx, |app, cx| {
                            app.update_composer_selection(this.session_id, cx, |s| {
                                s.set_model(model.clone())
                            });
                        });
                        let effort = this.app.read(cx).composer_selection(this.session_id).effort;
                        this.slider.update(cx, |slider, cx| {
                            slider.set_value(effort.index() as f32, window, cx)
                        });
                        cx.notify();
                    })),
            );
        }
        let body = v_flex()
            .id("session-model-body")
            .min_h_0()
            .overflow_y_scroll()
            .gap_2()
            .child(
                h_flex()
                    .px_2()
                    .pt_1()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(conduit::i18n::text_static(
                                "model_picker.model_configuration",
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(if pending {
                                conduit::i18n::text_static("files.new_session")
                            } else {
                                conduit::i18n::text_static("model_picker.current_session")
                            }),
                    ),
            )
            .child(self.engine_picker(cx))
            .child(channel_picker)
            .child(
                div()
                    .px_2()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::text_static("model_picker.model_98fd0c")),
            )
            .child(
                div()
                    .debug_selector(|| "session-model-search".into())
                    .child(Input::new(&self.search).h(px(30.))),
            )
            .child(list)
            .when(count == 0, |c| {
                c.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::text_static(
                            "model_picker.no_matching_models",
                        )),
                )
            })
            .when_some(note, |c, note| {
                c.child(
                    div()
                        .id("session-model-note")
                        .truncate()
                        .tooltip({
                            let note = note.clone();
                            move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(note.clone())
                                    .build(window, cx)
                            }
                        })
                        .px_2()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(note),
                )
            })
            .when(engine == Provider::OpenCode, |panel| {
                let picker = cx.entity();
                panel.child(
                    v_flex()
                        .px_2()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .child(conduit::i18n::text_static("model_picker.reasoning_effort")),
                        )
                        .child(
                            h_flex()
                                .id("opencode-effort-options")
                                .flex_wrap()
                                .max_h(px(72.))
                                .overflow_y_scroll()
                                .gap_1()
                                .children(
                                    std::iter::once((
                                        "default".to_string(),
                                        conduit::i18n::text_static("chat.default").to_string(),
                                    ))
                                    .chain(
                                        variants
                                            .into_iter()
                                            .filter(|(value, _)| value != "default"),
                                    )
                                    .enumerate()
                                    .map(
                                        move |(index, (value, label))| {
                                            let selected =
                                                variant.as_deref().unwrap_or("default") == value;
                                            let picker = picker.clone();
                                            Button::new(("opencode-effort", index))
                                                .small()
                                                .ghost()
                                                .child(
                                                    div()
                                                        .debug_selector(move || {
                                                            format!("opencode-effort-{index}")
                                                        })
                                                        .child(label),
                                                )
                                                .disabled(disabled)
                                                .cursor_pointer()
                                                .when(selected, |b| {
                                                    b.bg(rgb(palette().surface_hover))
                                                })
                                                .on_click(move |_, _, cx| {
                                                    picker.update(cx, |this, cx| {
                                                        this.app.update(cx, |app, cx| {
                                                            app.update_composer_selection(
                                                                this.session_id,
                                                                cx,
                                                                |s| {
                                                                    s.opencode.variant =
                                                                        Some(value.clone());
                                                                },
                                                            );
                                                        });
                                                        cx.notify();
                                                    });
                                                })
                                        },
                                    ),
                                ),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().faint))
                                .child(conduit::i18n::text_static(
                                    "model_picker.available_levels_update_after_connecting_to_the",
                                )),
                        ),
                )
            })
            .when(engine != Provider::OpenCode, |panel| {
                panel.child(
                    v_flex()
                        .px_2()
                        .pt_3()
                        .pb_2()
                        .gap_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(
                            h_flex()
                                .items_center()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(conduit::i18n::text_static(
                                            "model_picker.reasoning_effort",
                                        )),
                                )
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .font_semibold()
                                        .text_color(rgb(palette().text))
                                        .child(effort.label()),
                                ),
                        )
                        .child(
                            div()
                                .debug_selector(|| "session-effort-control".into())
                                .w_full()
                                .child(
                                    Slider::new(&self.slider)
                                        .w_full()
                                        .disabled(disabled)
                                        .accessibility_label(conduit::i18n::text_static(
                                            "model_picker.model_reasoning_effort",
                                        )),
                                ),
                        )
                        .child(h_flex().w_full().justify_between().children(
                            ReasoningEffort::ALL.map(|level| {
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().faint))
                                    .child(level.label())
                            }),
                        ))
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .line_height(relative(1.4))
                                .text_color(rgb(palette().faint))
                                .child(conduit::i18n::text_static(
                                    "model_picker.higher_effort_may_take_longer_supported_levels",
                                )),
                        ),
                )
            });
        v_flex()
            .debug_selector(|| "session-model-panel".into())
            .w(px(340.))
            .max_h((window.viewport_size().height - px(200.)).max(px(180.)))
            .gap_2()
            .child(body)
            .child(
                h_flex()
                    .debug_selector(|| "session-model-footer".into())
                    .flex_shrink_0()
                    .px_2()
                    .pt_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(if pending {
                                conduit::i18n::text_static(
                                    "model_picker.the_first_message_starts_a_session_with",
                                )
                            } else {
                                conduit::i18n::text_static(
                                    "model_picker.applies_to_the_next_message_switching_engines",
                                )
                            }),
                    )
                    .child(
                        Button::new("session-model-refresh")
                            .ghost()
                            .small()
                            .label(conduit::i18n::text_static("model_picker.refresh"))
                            .cursor_pointer()
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.app.update(cx, |app, cx| {
                                    app.reload_all_models_async(true, window, cx)
                                });
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl SessionModelPicker {
    fn engine_picker(&self, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let provider = app.new_session_provider;
        let disabled = !app.writable || app.closing;
        let view = cx.entity();
        let trigger = Button::new("engine-picker-trigger")
            .ghost()
            .w_full()
            .h(px(34.))
            .px_2()
            .disabled(disabled)
            .cursor_pointer()
            .accessibility_label(conduit::i18n::text_static("model_picker.select_cli_engine"))
            .child(
                h_flex()
                    .debug_selector(|| "session-engine-control".into())
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(conduit::i18n::text_static("model_picker.engine")),
                    )
                    .child(provider_icon(provider))
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().text))
                            .child(provider.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("engine-picker")
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(300.)).gap_1();
                for (index, option) in Provider::ALL.into_iter().enumerate() {
                    let enabled = view.read(cx).app.read(cx).store.cli.enabled(option);
                    let picker = view.clone();
                    let popover = popover.clone();
                    let active = option == provider;
                    list = list.child(
                        Button::new(match option {
                            Provider::Claude => "engine-option-claude",
                            Provider::Codex => "engine-option-codex",
                            Provider::OpenCode => "engine-option-opencode",
                        })
                        .ghost()
                        .h(px(34.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .disabled(!enabled)
                        .child(
                            h_flex()
                                .debug_selector(move || format!("session-engine-option-{index}"))
                                .w_full()
                                .gap_2()
                                .items_center()
                                .child(provider_icon(option))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().muted))
                                        .child(if enabled {
                                            option.label().to_owned()
                                        } else {
                                            conduit::i18n::format(
                                                "model_picker.disabled",
                                                &[("arg0", option.label().to_string())],
                                            )
                                        }),
                                )
                                .when(active, |row| row.child(Icon::new(IconName::Check).small())),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, window, cx| {
                            picker.update(cx, |this, cx| {
                                this.app
                                    .update(cx, |app, cx| app.select_engine(option, window, cx));
                                this.search.update(cx, |s, cx| s.set_value("", window, cx));
                                let effort =
                                    this.app.read(cx).composer_selection(this.session_id).effort;
                                this.slider.update(cx, |slider, cx| {
                                    slider.set_value(effort.index() as f32, window, cx)
                                });
                                cx.notify();
                            });
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list
            })
            .into_any_element()
    }
}

impl ConduitApp {
    /// Resolve the same destination used by sending: a selected conversation
    /// for this engine, or the configuration for the next new conversation.
    pub(super) fn model_picker_session_id(&self, session_id: u64) -> Option<u64> {
        self.selected_session
            .filter(|id| *id == session_id)
            .filter(|id| {
                self.session(*id)
                    .is_some_and(|s| s.provider == self.new_session_provider)
            })
    }

    pub(super) fn composer_selection(&self, session_id: u64) -> ComposerSelection {
        if let Some(session) = self
            .model_picker_session_id(session_id)
            .and_then(|id| self.session(id))
        {
            return ComposerSelection::from_session(session);
        }
        self.pending_model_selection
            .as_ref()
            .filter(|s| s.engine == self.new_session_provider)
            .cloned()
            .unwrap_or_else(|| self.default_selection(self.new_session_provider))
    }

    pub(super) fn update_composer_selection(
        &mut self,
        session_id: u64,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut ComposerSelection),
    ) {
        let target = self.model_picker_session_id(session_id);
        if !self.writable || self.closing || target.is_some_and(|id| self.session_busy(id)) {
            return;
        }
        let mut selection = self.composer_selection(session_id);
        update(&mut selection);
        if let Some(session) = target.and_then(|id| self.session_mut(id)) {
            selection.apply(session);
            self.persist();
        } else {
            self.pending_model_selection = Some(selection);
        }
        cx.notify();
    }

    /// Session permission selector in the composer. FullAccess is
    /// visually flagged (warning colors + alert icon) because it removes the
    /// CLI's own safety gates for this session.
    pub(super) fn permission_picker(&self, view: Entity<Self>) -> AnyElement {
        let permission = self
            .send_target(true)
            .and_then(|id| self.session(id))
            .map(|session| session.permission)
            .unwrap_or(self.pending_permission);
        let opencode = self.new_session_provider == Provider::OpenCode;
        let warning = permission == PermissionLevel::FullAccess;
        let trigger = Button::new("permission-picker-trigger")
            .ghost()
            .small()
            .h(px(28.))
            .px_1()
            .cursor_pointer()
            .tooltip(match permission {
                PermissionLevel::ReadOnly => {
                    conduit::i18n::text_static("model_picker.session_permissions_read_only")
                }
                PermissionLevel::WorkspaceWrite => {
                    conduit::i18n::text_static("model_picker.session_permissions_workspace_write")
                }
                PermissionLevel::FullAccess => conduit::i18n::text_static(
                    "model_picker.session_permissions_full_access_warning_no_sandbox",
                ),
            })
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(if warning {
                            gpui_kit::assets::IconName::TriangleAlert
                        } else {
                            gpui_kit::assets::IconName::Shield
                        })
                        .small()
                        .text_color(rgb(if warning {
                            palette().warning_text
                        } else {
                            palette().faint
                        })),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if warning {
                                palette().warning_text
                            } else {
                                palette().muted
                            }))
                            .child(permission.label()),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("permission-picker")
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                for option in [
                    PermissionLevel::ReadOnly,
                    PermissionLevel::WorkspaceWrite,
                    PermissionLevel::FullAccess,
                ] {
                    let app = view.clone();
                    let popover = popover.clone();
                    let active = option == permission;
                    let is_danger = option == PermissionLevel::FullAccess;
                    list = list.child(
                        Button::new(match option {
                            PermissionLevel::ReadOnly => "permission-option-readonly",
                            PermissionLevel::WorkspaceWrite => "permission-option-workspace",
                            PermissionLevel::FullAccess => "permission-option-full",
                        })
                        .ghost()
                        .h(px(52.))
                        .w_full()
                        .px_2()
                        .cursor_pointer()
                        .accessibility_label(option.label())
                        .child(
                            v_flex()
                                .w_full()
                                .gap(px(2.))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .gap_2()
                                        .items_center()
                                        .child(
                                            Icon::new(if is_danger {
                                                gpui_kit::assets::IconName::TriangleAlert
                                            } else {
                                                match option {
                                                    PermissionLevel::ReadOnly => {
                                                        gpui_kit::assets::IconName::Eye
                                                    }
                                                    _ => gpui_kit::assets::IconName::SquarePen,
                                                }
                                            })
                                            .small()
                                            .text_color(rgb(if is_danger {
                                                palette().warning_text
                                            } else {
                                                palette().muted
                                            })),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(12.))
                                                .text_color(rgb(if is_danger {
                                                    palette().warning_text
                                                } else {
                                                    palette().muted
                                                }))
                                                .child(option.label()),
                                        ),
                                )
                                .child(
                                    div()
                                        .pl_6()
                                        .text_size(ui_text_size(12.))
                                        .text_color(rgb(palette().faint))
                                        .child(match (opencode, option) {
                                            (true, PermissionLevel::ReadOnly) => {
                                                conduit::i18n::text_static("model_picker.opencode_read_tools_only_commands_and_writes")
                                            }
                                            (true, PermissionLevel::WorkspaceWrite) => {
                                                conduit::i18n::text_static("model_picker.opencode_writes_commands_and_access_outside_the")
                                            }
                                            (_, PermissionLevel::ReadOnly) => {
                                                conduit::i18n::text_static("model_picker.codex_read_only_sandbox_claude_read_tools")
                                            }
                                            (_, PermissionLevel::WorkspaceWrite) => {
                                                conduit::i18n::text_static("model_picker.codex_workspace_sandbox_claude_automatically_accept_file")
                                            }
                                            (_, PermissionLevel::FullAccess) => {
                                                conduit::i18n::text_static("model_picker.warning_no_sandbox_restrictions_commands_run_without")
                                            }
                                        }),
                                ),
                        )
                        .when(active, |button| button.bg(rgb(palette().surface_hover)))
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.set_session_permission(option, window, cx)
                            });
                            if let Some(popover) = popover.clone().upgrade() {
                                popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            }
                        }),
                    );
                }
                list.child(
                    div()
                        .px_2()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(
                            conduit::i18n::text_static("model_picker.full_access_immediately_handles_pending_approvals_automatically"),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn set_session_permission(
        &mut self,
        permission: PermissionLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let target = self.send_target(true);
        if let Some(id) = target
            && let Some(session) = self.session_mut(id)
            && session.permission != permission
        {
            session.permission = permission;
            self.persist();
        } else if target.is_none() {
            self.pending_permission = permission;
        }
        if permission == PermissionLevel::FullAccess
            && let Some(run) = target.and_then(|id| self.session_control.runs.get(&id))
        {
            run.runtime.enable_full_access();
        }
        // Sandbox changes apply to the next turn. Explicit full access also
        // resolves the active turn's approval gates, without restarting it.
        if self
            .selected_session
            .is_none_or(|id| !self.session_control.runs.contains_key(&id))
        {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn composer_model_picker(
        &self,
        view: Entity<Self>,
        session_id: u64,
        disabled: bool,
    ) -> AnyElement {
        let selection = self.composer_selection(session_id);
        let provider = selection.engine;
        let selected_model = selection.model;
        let (_, mut models) = self.channel_models(provider, &selection.channel);
        if provider == Provider::OpenCode && models.is_empty() {
            models = self
                .native_model_preferences(provider)
                .merge(&selection.opencode.models(), false);
        }
        let label = selected_model
            .as_ref()
            .map(|id| {
                models
                    .iter()
                    .find(|m| m.id == *id)
                    .map(|m| m.label.clone())
                    .unwrap_or_else(|| id.clone())
            })
            .unwrap_or_else(|| conduit::i18n::text_static("model_picker.default_model").into());
        let channel = if provider == Provider::OpenCode && selection.channel == ModelChannel::Cli {
            selected_model
                .as_deref()
                .and_then(|id| id.split_once('/'))
                .map(|(provider, _)| provider.to_owned())
                .unwrap_or_else(|| self.channel_label(provider, &selection.channel))
        } else {
            self.channel_label(provider, &selection.channel)
        };
        let effort = if provider == Provider::OpenCode {
            selection
                .opencode
                .effective_variant()
                .map(str::to_owned)
                .unwrap_or_else(|| conduit::i18n::text_static("chat.default").into())
        } else {
            selection.effort.label().to_owned()
        };
        let trigger = Button::new("composer-model-trigger")
            .ghost()
            .small()
            .min_w_0()
            .max_w_full()
            .h(px(26.))
            .px_2()
            .rounded(px(7.))
            .disabled(disabled)
            .when(!disabled, |b| b.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_engine_provider_model_and_reasoning_effort",
            ))
            .tooltip(conduit::i18n::format(
                "model_picker.reasoning_effort_5315ac",
                &[
                    ("arg0", provider.label().to_string()),
                    ("channel", channel.to_string()),
                    ("effort", effort.to_string()),
                ],
            ))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .debug_selector(|| "composer-model-control".into())
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().faint))
                            .child(provider.label()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_shrink_1()
                            .max_w(px(170.))
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(label),
                    )
                    .when(
                        if provider == Provider::OpenCode {
                            selection.opencode.effective_variant().is_some()
                        } else {
                            selection.effort != ReasoningEffort::Default
                        },
                        |row| {
                            row.child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().faint))
                                    .child(effort.clone()),
                            )
                        },
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("composer-model-popover")
            .trigger_style(StyleRefinement::default().min_w_0().max_w_full())
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, window, cx| {
                view.update(cx, |app, cx| {
                    if app.new_session_provider == Provider::OpenCode {
                        app.ensure_opencode_models(false, cx);
                    }
                });
                let picker = window.use_keyed_state(
                    ("session-model-picker", session_id),
                    cx,
                    |window, cx| SessionModelPicker::new(view.clone(), session_id, window, cx),
                );
                div().child(picker)
            })
            .into_any_element()
    }

    pub(super) fn channel_label(&self, engine: Provider, channel: &ModelChannel) -> String {
        match channel {
            ModelChannel::Cli => if engine == Provider::OpenCode {
                conduit::i18n::text_static("model_picker.opencode_native")
            } else {
                conduit::i18n::text_static("generation.cli_default")
            }
            .into(),
            ModelChannel::Ccs(id) => self.channel_label(
                engine,
                &ModelChannel::Provider(ModelProvider::ccs_id(engine, id)),
            ),
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| {
                    conduit::i18n::text_static("model_picker.provider_unavailable").into()
                }),
        }
    }

    pub(super) fn channel_models(
        &self,
        engine: Provider,
        channel: &ModelChannel,
    ) -> (Option<String>, Vec<models::ModelOption>) {
        if engine == Provider::OpenCode {
            return self
                .engine_catalog(engine)
                .map(|catalog| {
                    (
                        catalog.error.clone().or_else(|| catalog.note.clone()),
                        self.managed_native_models(engine, false),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        Some(
                            conduit::i18n::text_static(
                                "model_picker.loading_the_native_model_catalog_from_opencode",
                            )
                            .into(),
                        ),
                        self.managed_native_models(engine, false),
                    )
                });
        }
        match channel {
            ModelChannel::Cli => self
                .catalogs
                .get(&engine)
                .map(|c| {
                    (
                        c.error.clone().or_else(|| c.note.clone()),
                        self.managed_native_models(engine, false),
                    )
                })
                .unwrap_or_else(|| (None, self.managed_native_models(engine, false))),
            ModelChannel::Ccs(id) => self.channel_models(
                engine,
                &ModelChannel::Provider(ModelProvider::ccs_id(engine, id)),
            ),
            ModelChannel::Provider(id) => {
                let models = vec![];
                if let Some(p) = self
                    .store
                    .providers
                    .iter()
                    .find(|p| p.id == *id && p.engine == engine)
                {
                    (None, p.catalog())
                } else {
                    (
                        Some(
                            conduit::i18n::text_static(
                                "model_picker.provider_removed_select_another_provider",
                            )
                            .into(),
                        ),
                        models,
                    )
                }
            }
        }
    }

    pub(super) fn session_route(&self, id: u64) -> anyhow::Result<RuntimeRoute> {
        let s = self.session(id).ok_or_else(|| {
            anyhow::anyhow!(conduit::i18n::text("model_picker.session_does_not_exist"))
        })?;
        let mut route = self.channel_route(s.provider, &s.channel)?;
        if s.channel == ModelChannel::Cli {
            route.opencode_variant = s.opencode.variant.clone();
        }
        Ok(route)
    }

    pub(super) fn channel_route(
        &self,
        engine: Provider,
        channel: &ModelChannel,
    ) -> anyhow::Result<RuntimeRoute> {
        match channel {
            ModelChannel::Cli => Ok(RuntimeRoute::default()),
            ModelChannel::Ccs(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == ModelProvider::ccs_id(engine, id) && p.engine == engine)
                .ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "model_picker.provider_not_synced_refresh_model_configuration"
                    ))
                })?
                .runtime_route(&self.ccs.database_path),
            ModelChannel::Provider(id) => self
                .store
                .providers
                .iter()
                .find(|p| p.id == *id && p.engine == engine)
                .ok_or_else(|| {
                    anyhow::anyhow!(conduit::i18n::text(
                        "model_picker.provider_removed_select_another_provider"
                    ))
                })?
                .runtime_route(&self.ccs.database_path),
        }
    }

    /// Model catalog merged with cc-switch synced models, for the Git
    /// generation model picker.
    pub(super) fn git_generation_catalog(&self, provider: Provider) -> ModelCatalog {
        if provider == Provider::OpenCode {
            return self.engine_catalog(provider).cloned().unwrap_or_default();
        }
        let catalog = self.catalogs.get(&provider);
        let synced = self.ccs.current(provider);
        match (catalog, synced) {
            (Some(catalog), Some(synced)) if !synced.models.is_empty() => {
                catalog.with_synced(&format!("CCS · {}", synced.name), &synced.models)
            }
            (Some(catalog), _) => catalog.clone(),
            (None, Some(synced)) if !synced.models.is_empty() => ModelCatalog::default()
                .with_synced(&format!("CCS · {}", synced.name), &synced.models),
            (None, _) => ModelCatalog::default(),
        }
    }

    /// Picker for the CLI engine used to auto-name sessions.
    pub(super) fn naming_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or(conduit::i18n::text_static(
                "generation.follow_current_session",
            ));
        let trigger = Button::new("naming-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_naming_engine",
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 4] = [
                    (
                        None,
                        conduit::i18n::text_static("generation.follow_current_session"),
                    ),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                    (Some(Provider::OpenCode), Provider::OpenCode.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("naming-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_provider = option;
                                    this.store.ui.naming_channel = None;
                                    this.store.ui.naming_model = None;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(ui_text_size(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Picker for what automatic session titling reacts to.
    pub(super) fn naming_mode_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.ui.naming_mode;
        let label = naming_mode_label(current);
        let trigger = Button::new("naming-mode-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_automatic_renaming_mode",
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-mode-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(240.)).gap(px(1.));
                for (index, (mode, name, detail)) in [
                    (
                        NamingMode::Latest,
                        conduit::i18n::text_static("model_picker.rename_from_the_latest_content"),
                        conduit::i18n::text_static(
                            "model_picker.update_the_title_after_each_turn_using",
                        ),
                    ),
                    (
                        NamingMode::FirstMessage,
                        conduit::i18n::text_static("model_picker.rename_from_the_first_message"),
                        conduit::i18n::text_static(
                            "model_picker.generate_a_title_once_after_the_first",
                        ),
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == mode;
                    list = list.child(
                        Button::new(("naming-mode-option", index))
                            .ghost()
                            .h(px(44.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.naming_mode = mode;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(ui_text_size(12.))
                                                    .text_color(rgb(if active {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(name),
                                            )
                                            .child(
                                                div()
                                                    .text_size(ui_text_size(12.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(detail),
                                            ),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Picker for the model used to auto-name sessions.
    pub(super) fn naming_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let (default_label, allow_default) = self.generation_model_defaults(GenerationKind::Naming);
        let provider = self
            .store
            .ui
            .naming_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.generation_catalog(GenerationKind::Naming);
        let selected_model = self.store.ui.naming_model.clone();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| default_label.to_string());
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("naming-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_naming_model",
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("naming-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("naming-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let options: Vec<(Option<String>, String, String)> = allow_default
                    .then(|| (None, default_label.to_string(), String::new()))
                    .into_iter()
                    .chain(models.iter().map(|model| {
                        (
                            Some(model.id.clone()),
                            model.label.clone(),
                            model.note.clone(),
                        )
                    }))
                    .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    let mark = model_mark(provider, id.as_deref());
                    list = list.child(
                        Button::new(("naming-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(conduit::i18n::format(
                                "model_picker.model",
                                &[("label", label.to_string())],
                            ))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.set_generation_model(GenerationKind::Naming, id);
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(mark)
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(ui_text_size(12.))
                                                    .text_color(rgb(if selected {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(ui_text_size(12.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::format(
                            "model_picker.models",
                            &[
                                ("provider_label", provider_label.to_string()),
                                ("arg0", format!("{}", models.len())),
                            ],
                        )),
                );
                list
            })
            .into_any_element()
    }

    /// Settings picker for the CLI used to generate commit messages.
    pub(super) fn git_provider_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let current = self.store.git.generation_provider;
        let label = current
            .map(|provider| provider.label())
            .unwrap_or(conduit::i18n::text_static(
                "generation.follow_current_session",
            ));
        let trigger = Button::new("git-provider-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_generation_cli",
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when_some(current, |row, provider| row.child(provider_icon(provider)))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-provider-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex().w(px(220.)).gap(px(1.));
                let options: [(Option<Provider>, &str); 4] = [
                    (
                        None,
                        conduit::i18n::text_static("generation.follow_current_session"),
                    ),
                    (Some(Provider::Claude), Provider::Claude.label()),
                    (Some(Provider::Codex), Provider::Codex.label()),
                    (Some(Provider::OpenCode), Provider::OpenCode.label()),
                ];
                for (index, (option, name)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let active = current == option;
                    list = list.child(
                        Button::new(("git-provider-option", index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(active, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.git.generation_provider = option;
                                    this.store.git.generation_channel = None;
                                    this.store.git.generation_model = None;
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .when_some(option, |row, provider| {
                                        row.child(provider_icon(provider))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(ui_text_size(12.))
                                            .text_color(rgb(if active {
                                                palette().text
                                            } else {
                                                palette().muted
                                            }))
                                            .child(name),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }

    /// Settings picker for the model used to generate commit messages.
    pub(super) fn git_model_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        let (default_label, allow_default) = self.generation_model_defaults(GenerationKind::Git);
        let provider = self
            .store
            .git
            .generation_provider
            .or_else(|| {
                self.selected_session
                    .and_then(|id| self.session(id))
                    .map(|session| session.provider)
            })
            .unwrap_or(self.new_session_provider);
        let merged = self.generation_catalog(GenerationKind::Git);
        let selected_model = self.store.git.generation_model.clone();
        let trigger_label = selected_model
            .as_deref()
            .map(|id| merged.label(id))
            .unwrap_or_else(|| default_label.to_string());
        let models = merged.models;
        let provider_label = provider.label();
        let trigger = Button::new("git-model-trigger")
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |button| button.cursor_pointer())
            .accessibility_label(conduit::i18n::text_static(
                "model_picker.select_generation_model",
            ))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(model_mark(provider, selected_model.as_deref()))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(if disabled {
                                palette().faint
                            } else {
                                palette().muted
                            }))
                            .child(trigger_label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new("git-model-popover")
            .anchor(Anchor::BottomRight)
            .offset(px(8.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id("git-model-list")
                    .w(px(320.))
                    .gap(px(1.))
                    .max_h(px(320.))
                    .overflow_y_scroll();
                let options: Vec<(Option<String>, String, String)> = allow_default
                    .then(|| (None, default_label.to_string(), String::new()))
                    .into_iter()
                    .chain(models.iter().map(|model| {
                        (
                            Some(model.id.clone()),
                            model.label.clone(),
                            model.note.clone(),
                        )
                    }))
                    .collect();
                for (index, (id, label, note)) in options.into_iter().enumerate() {
                    let view = view.clone();
                    let popover = popover.clone();
                    let selected = selected_model == id;
                    let mark = model_mark(provider, id.as_deref());
                    list = list.child(
                        Button::new(("git-model-option", index))
                            .ghost()
                            .h(px(38.))
                            .w_full()
                            .px_2()
                            .rounded(px(8.))
                            .cursor_pointer()
                            .accessibility_label(conduit::i18n::format(
                                "model_picker.model",
                                &[("label", label.to_string())],
                            ))
                            .when(selected, |button| button.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                let id = id.clone();
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.set_generation_model(GenerationKind::Git, id);
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .items_center()
                                    .child(mark)
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(1.))
                                            .child(
                                                div()
                                                    .truncate()
                                                    .text_size(ui_text_size(12.))
                                                    .text_color(rgb(if selected {
                                                        palette().text
                                                    } else {
                                                        palette().muted
                                                    }))
                                                    .child(label),
                                            )
                                            .when(!note.is_empty(), |column| {
                                                column.child(
                                                    div()
                                                        .truncate()
                                                        .text_size(ui_text_size(12.))
                                                        .text_color(rgb(palette().faint))
                                                        .child(note),
                                                )
                                            }),
                                    )
                                    .when(selected, |row| {
                                        row.child(
                                            Icon::new(IconName::Check)
                                                .small()
                                                .text_color(rgb(palette().muted)),
                                        )
                                    }),
                            ),
                    );
                }
                list = list.child(
                    div()
                        .px_2()
                        .pt_1()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::format(
                            "model_picker.models",
                            &[
                                ("provider_label", provider_label.to_string()),
                                ("arg0", format!("{}", models.len())),
                            ],
                        )),
                );
                list
            })
            .into_any_element()
    }
}
