//! Persistent defaults apply only when constructing a new conversation.
use super::*;
mod model_picker;

/// Resolve a legacy CCS alias only for display, and only to an existing
/// registry entry for the same engine. Never migrate the persisted selection.
fn default_channel_for_display(
    engine: Provider,
    channel: &ModelChannel,
    providers: &[crate::providers::ModelProvider],
) -> ModelChannel {
    let ModelChannel::Ccs(source_id) = channel else {
        return channel.clone();
    };
    let registry_id = crate::providers::ModelProvider::ccs_id(engine, source_id);
    if providers
        .iter()
        .any(|provider| provider.engine == engine && provider.id == registry_id)
    {
        ModelChannel::Provider(registry_id)
    } else {
        channel.clone()
    }
}

impl ConduitApp {
    pub(in crate::app) fn default_selection(&self, engine: Provider) -> ComposerSelection {
        let mut selection = ComposerSelection::new(engine);
        if let Some(defaults) = self.store.session_defaults.get(&engine) {
            selection.channel = defaults.channel.clone();
            selection.model = defaults.model.clone();
        }
        selection
    }

    /// Re-selecting the displayed service preserves the saved model and legacy route.
    pub(super) fn set_default_channel(
        &mut self,
        engine: Provider,
        channel: ModelChannel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let current = self.default_selection(engine);
        if default_channel_for_display(engine, &current.channel, &self.store.providers) == channel {
            return false;
        }
        self.set_service_default(engine, channel, None, window, cx)
    }

    /// Compatibility entry point; defaults share the management browsing engine.
    pub(super) fn settings_defaults_page(&self, disabled: bool, view: Entity<Self>) -> SettingPage {
        settings_page(
            conduit::i18n::text_static("settings.defaults.new_session_defaults"),
            FullIconName::Settings,
        )
        .group(self.settings_new_session_defaults_group(disabled, view))
    }

    /// Render one engine's defaults without changing the browsing engine or a session.
    pub(super) fn settings_new_session_defaults_group(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingGroup {
        let engine = self.connection_settings.engine;
        let disabled = disabled || self.connection_settings.saving;
        let selection = self.default_selection(engine);
        let mut channels = vec![(
            ModelChannel::Cli,
            conduit::i18n::text_static("settings.defaults.native_cli_configuration").to_string(),
        )];
        channels.extend(
            self.store
                .providers
                .iter()
                .filter(|p| p.engine == engine)
                .map(|p| (ModelChannel::Provider(p.id.clone()), p.name.clone())),
        );
        let display_channel =
            default_channel_for_display(engine, &selection.channel, &self.store.providers);
        let selected_channel = channels
            .iter()
            .find(|(channel, _)| *channel == display_channel);
        let service_unavailable = selected_channel.is_none();
        let label = selected_channel
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| conduit::i18n::text("settings.navigation.service_unavailable"));
        let service_warning = service_unavailable.then(|| {
            let id = match &selection.channel {
                ModelChannel::Provider(id) | ModelChannel::Ccs(id) => id.as_str(),
                ModelChannel::Cli => "",
            };
            conduit::i18n::format(
                "settings.sessions.service_unavailable_description",
                &[("id", id.into())],
            )
        });
        let route_view = view.clone();
        let route = SettingField::render(move |_, _, _| {
            let channels = channels.clone();
            let display_channel = display_channel.clone();
            let view = route_view.clone();
            Button::new(("default-channel", engine as usize))
                .debug_selector(move || format!("default-channel-{}", engine as usize))
                .outline()
                .small()
                .max_w(rems(14.))
                .label(label.clone())
                .tooltip(label.clone())
                .icon(FullIconName::ChevronDown)
                .disabled(disabled)
                .dropdown_menu(move |mut menu, _, _| {
                    for (channel, label) in &channels {
                        let channel = channel.clone();
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(channel == display_channel)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |app, cx| {
                                        app.set_default_channel(
                                            engine,
                                            channel.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                        );
                    }
                    menu
                })
        });
        let route = SettingItem::new(
            conduit::i18n::text_static("settings.sessions.default_service"),
            route,
        );
        let route = if let Some(warning) = service_warning {
            route.description(settings_description(warning))
        } else {
            route
        };
        let mut models = vec![(
            None,
            conduit::i18n::text_static("settings.defaults.use_service_default_model").to_string(),
        )];
        // Catalog absence is presentation state, never a reason to rewrite a saved ID.
        let catalog = self.channel_models(engine, &selection.channel).1;
        models.extend(
            catalog
                .into_iter()
                .map(|model| (Some(model.id), model.label)),
        );
        let selected_model = models.iter().find(|(model, _)| *model == selection.model);
        // Catalogs do not expose a uniform authoritative Loaded state; a
        // missing entry can mean an unloaded, incomplete, or failed catalog.
        let model_unconfirmed = selected_model.is_none();
        let label = selected_model
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| selection.model.clone().unwrap_or_default());
        let selected_model_id = selection.model.clone();
        if model_unconfirmed {
            models.push((selected_model_id.clone(), label.clone()));
        }
        let model_view = view.clone();
        let model = SettingField::render(move |_, _, _| model_picker::DefaultModelPicker {
            app: model_view.clone(),
            engine,
            channel: selection.channel.clone(),
            choices: models
                .iter()
                .map(|(id, label)| model_picker::ModelChoice {
                    id: id.clone(),
                    label: label.clone(),
                })
                .collect(),
            selected: selected_model_id.clone(),
            disabled: disabled || service_unavailable,
        });
        let model = SettingItem::new(
            conduit::i18n::text_static("settings.sessions.default_model"),
            model,
        );
        let model = if service_unavailable || model_unconfirmed {
            model.description(settings_description(conduit::i18n::text_static(
                if service_unavailable {
                    "settings.sessions.model_needs_service_description"
                } else {
                    "settings.sessions.model_pending_confirmation_description"
                },
            )))
        } else {
            model
        };
        settings_section(format!(
            "{} · {}",
            conduit::i18n::text_static("settings.defaults.new_session_defaults"),
            engine.label(),
        ))
        .items(vec![
            SettingItem::new(
                conduit::i18n::text_static("settings.models.service_engine"),
                SettingField::render(move |_, _, _| {
                    super::management::engine_tabs(engine, false, view.clone())
                }),
            )
            .layout(Axis::Vertical),
            route,
            model,
        ])
        .footer(|_, _| conduit::i18n::text_static("settings.sessions.defaults_scope"))
    }

    pub(super) fn provider_configuration_references(&self, id: &str) -> Vec<String> {
        let channel = ModelChannel::Provider(id.into());
        let mut references = vec![];
        for (engine, defaults) in &self.store.session_defaults {
            if defaults.channel == channel {
                references.push(conduit::i18n::format(
                    "settings.defaults.new_session_defaults_4cc8f4",
                    &[("arg0", engine.label().to_string())],
                ));
            }
        }
        if self.generation_uses_provider(id)
            && self.store.ui.naming_channel.as_ref() == Some(&channel)
        {
            references
                .push(conduit::i18n::text_static("settings.defaults.automatic_naming").into());
        }
        if self.store.git.generation_channel.as_ref() == Some(&channel) {
            references.push(conduit::i18n::text_static("git_ui.git_commit_message").into());
        }
        references
    }

    pub(super) fn provider_references(&self, id: &str) -> Vec<String> {
        let channel = ModelChannel::Provider(id.into());
        let mut references = self.provider_configuration_references(id);
        for workspace in &self.store.workspaces {
            for session in &workspace.sessions {
                if session.channel == channel {
                    references.push(format!("{} / {}", workspace.name(), session.title));
                }
            }
        }
        references
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{
        ConduitApp, ModelChannel, Provider, Store, SystemTime, UNIX_EPOCH,
        default_channel_for_display,
    };
    use gpui_kit::{AppContext as _, TestAppContext};

    #[test]
    fn legacy_ccs_default_display_resolves_existing_engine_registry_without_migration() {
        let _locale = crate::test_support::chinese();
        let source_id = "shared-source-id";
        let mut store = Store::default();
        store.session_defaults.insert(
            Provider::Codex,
            crate::store::SessionDefaults {
                channel: ModelChannel::Ccs(source_id.into()),
                model: Some("saved-model-id".into()),
            },
        );
        // Legacy defaults survive serialization and the real startup CCS sync.
        let mut store: Store =
            serde_json::from_value(serde_json::to_value(store).unwrap()).unwrap();
        let saved_defaults = serde_json::to_value(&store.session_defaults).unwrap();
        let catalog = crate::ccs::CcsCatalog {
            providers: [Provider::Codex, Provider::Claude]
                .into_iter()
                .map(|engine| crate::ccs::CcsProvider {
                    provider: engine,
                    name: format!("{} legacy service", engine.label()),
                    id: source_id.into(),
                    current: false,
                    models: vec![],
                    base_url: "https://ccs-fixture.invalid/v1".into(),
                })
                .collect(),
            ..Default::default()
        };
        store.sync_ccs_providers(&catalog);
        assert_eq!(
            serde_json::to_value(&store.session_defaults).unwrap(),
            saved_defaults,
        );
        let snapshot = serde_json::to_value(&store).unwrap();
        let saved = &store.session_defaults[&Provider::Codex];
        let display =
            default_channel_for_display(Provider::Codex, &saved.channel, &store.providers);
        let registry_id = crate::providers::ModelProvider::ccs_id(Provider::Codex, source_id);
        assert_eq!(display, ModelChannel::Provider(registry_id.clone()));
        let service = store
            .providers
            .iter()
            .find(|provider| provider.engine == Provider::Codex && provider.id == registry_id)
            .unwrap();
        assert_eq!(service.name, "Codex legacy service");
        assert_eq!(
            default_channel_for_display(Provider::Codex, &display, &store.providers),
            display,
        );
        assert_eq!(
            default_channel_for_display(Provider::Codex, &ModelChannel::Cli, &store.providers),
            ModelChannel::Cli,
        );

        // The same source ID from another engine cannot satisfy the saved route.
        let other_engine_only: Vec<_> = store
            .providers
            .iter()
            .filter(|provider| provider.engine == Provider::Claude)
            .cloned()
            .collect();
        assert_eq!(other_engine_only.len(), 1);
        assert_eq!(
            default_channel_for_display(Provider::Codex, &saved.channel, &other_engine_only),
            ModelChannel::Ccs(source_id.into()),
        );
        assert_eq!(
            default_channel_for_display(Provider::Codex, &saved.channel, &[]),
            ModelChannel::Ccs(source_id.into()),
        );
        assert_eq!(saved.channel, ModelChannel::Ccs(source_id.into()));
        assert_eq!(saved.model.as_deref(), Some("saved-model-id"));
        assert_eq!(serde_json::to_value(&store).unwrap(), snapshot);
    }

    #[gpui_kit::test]
    fn new_session_defaults_persist_and_explicit_drafts_win_without_changing_history(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let directory = std::env::temp_dir().join(format!(
            "conduit-defaults-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = directory.join("state.json");
        let test_state = state.clone();
        cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut store = Store::default();
                let original = crate::app::tests::session();
                let original_value = serde_json::to_value(&original).unwrap();
                store.workspaces.push(crate::domain::Workspace {
                    path: directory.clone(),
                    sessions: vec![original],
                });
                store.session_defaults.insert(
                    Provider::Codex,
                    crate::store::SessionDefaults {
                        channel: ModelChannel::Provider("custom-test".into()),
                        model: Some("configured-model".into()),
                    },
                );
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.persistence = crate::persistence::Persistence::new(test_state);
                app.writable = true;
                let id = app.create_session(0, Provider::Codex, cx).unwrap();
                assert_eq!(
                    app.session(id).unwrap().model.as_deref(),
                    Some("configured-model")
                );
                assert_eq!(
                    app.session(id).unwrap().channel,
                    ModelChannel::Provider("custom-test".into())
                );
                assert_eq!(
                    serde_json::to_value(app.session(1).unwrap()).unwrap(),
                    original_value
                );
                let mut draft = app.default_selection(Provider::Codex);
                draft.model = Some("explicit-model".into());
                app.pending_model_selection = Some(draft);
                let id = app.create_session(0, Provider::Codex, cx).unwrap();
                assert_eq!(
                    app.session(id).unwrap().model.as_deref(),
                    Some("explicit-model")
                );
                assert!(app.pending_model_selection.is_none());
                // Session creation queues an asynchronous save; wait for its disk receipt.
                app.persistence.flush();
                assert_eq!(app.persistence.error, None);
                let loaded: Store =
                    serde_json::from_slice(&std::fs::read(&state).unwrap()).unwrap();
                assert_eq!(
                    loaded.session_defaults[&Provider::Codex].model.as_deref(),
                    Some("configured-model")
                );
                assert!(
                    app.provider_references("custom-test")
                        .iter()
                        .any(|r| r.contains("新会话默认"))
                );
                app.writable = false;
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
        std::fs::remove_dir_all(directory).unwrap();
    }
}
