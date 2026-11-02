//! Independent engine/provider/model routes for background text generation.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum GenerationKind {
    Naming,
    Git,
}

impl ConduitApp {
    pub(super) fn generation_settings(
        &self,
        kind: GenerationKind,
    ) -> (Option<Provider>, Option<ModelChannel>, Option<String>) {
        match kind {
            GenerationKind::Naming => (
                self.store.ui.naming_provider,
                self.store.ui.naming_channel.clone(),
                self.store.ui.naming_model.clone(),
            ),
            GenerationKind::Git => (
                self.store.git.generation_provider,
                self.store.git.generation_channel.clone(),
                self.store.git.generation_model.clone(),
            ),
        }
    }

    pub(super) fn generation_selection(
        &self,
        kind: GenerationKind,
        session_id: Option<u64>,
    ) -> (Provider, ModelChannel, Option<String>) {
        let (engine, channel, model) = self.generation_settings(kind);
        let session = session_id.and_then(|id| self.session(id));
        let effective_engine = engine
            .or_else(|| session.map(|s| s.provider))
            .unwrap_or(self.new_session_provider);
        let follows_session = engine.is_none() && channel.is_none();
        let effective_channel = channel.unwrap_or_else(|| {
            if follows_session {
                session.map(|s| s.channel.clone()).unwrap_or_default()
            } else {
                ModelChannel::Cli
            }
        });
        let effective_model = model.or_else(|| {
            follows_session
                .then(|| session.and_then(|s| s.model.clone()))
                .flatten()
        });
        (effective_engine, effective_channel, effective_model)
    }

    pub(super) fn generation_request(
        &self,
        kind: GenerationKind,
        session_id: Option<u64>,
    ) -> anyhow::Result<(Provider, Option<String>, RuntimeRoute)> {
        let (engine, channel, model) = self.generation_selection(kind, session_id);
        Ok((engine, model, self.channel_route(engine, &channel)?))
    }

    pub(super) fn set_generation_channel(
        &mut self,
        kind: GenerationKind,
        engine: Provider,
        channel: Option<ModelChannel>,
        model: Option<String>,
    ) {
        match kind {
            GenerationKind::Naming => {
                if channel.is_some() {
                    self.store.ui.naming_provider = Some(engine);
                }
                self.store.ui.naming_channel = channel;
                self.store.ui.naming_model = model;
            }
            GenerationKind::Git => {
                if channel.is_some() {
                    self.store.git.generation_provider = Some(engine);
                }
                self.store.git.generation_channel = channel;
                self.store.git.generation_model = model;
            }
        }
    }

    pub(super) fn set_generation_model(&mut self, kind: GenerationKind, model: Option<String>) {
        if model.is_some() {
            // A fixed model belongs to the current engine/route. Snapshot both so
            // switching the active chat cannot silently change its credentials.
            let (engine, channel, _) = self.generation_selection(kind, self.selected_session);
            self.set_generation_channel(kind, engine, Some(channel), model);
        } else {
            match kind {
                GenerationKind::Naming => self.store.ui.naming_model = None,
                GenerationKind::Git => self.store.git.generation_model = None,
            }
        }
    }

    pub(super) fn generation_uses_provider(&self, id: &str) -> bool {
        [GenerationKind::Naming, GenerationKind::Git]
            .into_iter()
            .any(|kind| {
                let (engine, channel, _) = self.generation_settings(kind);
                match channel {
                    Some(ModelChannel::Provider(selected)) => selected == id,
                    Some(ModelChannel::Ccs(selected)) => {
                        engine.is_some_and(|engine| ModelProvider::ccs_id(engine, &selected) == id)
                    }
                    _ => false,
                }
            })
    }

    /// The model menu must reflect the chosen route, not another CCS provider.
    pub(super) fn generation_catalog(&self, kind: GenerationKind) -> ModelCatalog {
        let (engine, channel, model) = self.generation_selection(kind, self.selected_session);
        let (note, mut models) = if channel == ModelChannel::Cli {
            let catalog = self.git_generation_catalog(engine);
            (catalog.error.or(catalog.note), catalog.models)
        } else {
            self.channel_models(engine, &channel)
        };
        if engine == Provider::OpenCode
            && let Some(native) = model
                .as_deref()
                .and_then(|id| id.split_once('/').map(|(p, _)| p))
        {
            models.retain(|m| m.id.split_once('/').is_some_and(|(p, _)| p == native));
        }
        ModelCatalog {
            models,
            note,
            ..Default::default()
        }
    }

    pub(super) fn generation_model_defaults(&self, kind: GenerationKind) -> (&'static str, bool) {
        let (engine_override, channel_override, model_override) = self.generation_settings(kind);
        let (engine, channel, _) = self.generation_selection(kind, self.selected_session);
        let label = if engine_override.is_none() && channel_override.is_none() {
            "跟随当前会话"
        } else if channel == ModelChannel::Cli {
            "CLI 默认"
        } else {
            "Provider 默认"
        };
        // A native OpenCode provider is encoded in a concrete provider/model ID.
        // Clearing the model would discard the independently selected provider.
        let allow_default = !(engine == Provider::OpenCode
            && model_override.as_deref().is_some_and(|id| id.contains('/')));
        (label, allow_default)
    }

    pub(super) fn naming_channel_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        self.generation_channel_picker(GenerationKind::Naming, view, disabled)
    }

    pub(super) fn git_channel_picker(&self, view: Entity<Self>, disabled: bool) -> AnyElement {
        self.generation_channel_picker(GenerationKind::Git, view, disabled)
    }

    fn generation_channel_picker(
        &self,
        kind: GenerationKind,
        view: Entity<Self>,
        disabled: bool,
    ) -> AnyElement {
        let (engine_override, channel_override, _) = self.generation_settings(kind);
        let (engine, channel, model) = self.generation_selection(kind, self.selected_session);
        let follow_session = engine_override.is_none() && channel_override.is_none();
        let native = (engine == Provider::OpenCode)
            .then(|| {
                model
                    .as_deref()
                    .and_then(|id| id.split_once('/').map(|(p, _)| p.to_owned()))
            })
            .flatten();
        let scope = match kind {
            GenerationKind::Naming => "naming-channel",
            GenerationKind::Git => "git-channel",
        };
        let default_label = if engine_override.is_none() {
            "跟随当前会话"
        } else {
            "CLI 默认"
        };
        let label = if follow_session {
            format!(
                "跟随当前会话 · {}",
                native
                    .clone()
                    .unwrap_or_else(|| self.channel_label(engine, &channel))
            )
        } else {
            native
                .clone()
                .unwrap_or_else(|| self.channel_label(engine, &channel))
        };
        let mut options = vec![(None, None, default_label.to_string())];
        if engine_override.is_none() {
            options.push((
                Some(ModelChannel::Cli),
                None,
                self.channel_label(engine, &ModelChannel::Cli),
            ));
        }
        if engine == Provider::OpenCode {
            let mut providers = BTreeMap::new();
            for model in self.channel_models(engine, &ModelChannel::Cli).1 {
                if let Some((provider, _)) = model.id.split_once('/') {
                    providers.entry(provider.to_owned()).or_insert(model.id);
                }
            }
            options.extend(
                providers
                    .into_iter()
                    .map(|(provider, model)| (Some(ModelChannel::Cli), Some(model), provider)),
            );
        } else {
            options.extend(
                self.store
                    .providers
                    .iter()
                    .filter(|p| p.engine == engine)
                    .map(|p| {
                        (
                            Some(ModelChannel::Provider(p.id.clone())),
                            None,
                            p.name.clone(),
                        )
                    }),
            );
        }
        let trigger = Button::new(SharedString::from(format!("{scope}-trigger")))
            .ghost()
            .small()
            .h(px(24.))
            .px_2()
            .rounded_full()
            .disabled(disabled)
            .when(!disabled, |b| b.cursor_pointer())
            .accessibility_label(match kind {
                GenerationKind::Naming => "选择命名 Provider",
                GenerationKind::Git => "选择生成 Provider",
            })
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        div()
                            .max_w(px(220.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(label),
                    )
                    .child(
                        Icon::new(IconName::ChevronDown)
                            .small()
                            .text_color(rgb(palette().faint)),
                    ),
            );
        Popover::new(SharedString::from(format!("{scope}-popover")))
            .anchor(Anchor::BottomRight)
            .offset(px(6.))
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let mut list = v_flex()
                    .id(SharedString::from(format!("{scope}-list")))
                    .w(px(280.))
                    .gap_1()
                    .max_h(px(320.))
                    .overflow_y_scroll();
                for (index, (option, option_model, label)) in options.iter().enumerate() {
                    let selected = if let Some(option_native) = option_model
                        .as_deref()
                        .and_then(|id| id.split_once('/').map(|(p, _)| p))
                    {
                        !follow_session && native.as_deref() == Some(option_native)
                    } else {
                        channel_override == *option
                            && (native.is_none() || (option.is_none() && follow_session))
                    };
                    let view = view.clone();
                    let popover = popover.clone();
                    let option = option.clone();
                    let option_model = option_model.clone();
                    list = list.child(
                        Button::new((scope, index))
                            .ghost()
                            .h(px(32.))
                            .w_full()
                            .px_2()
                            .rounded(px(6.))
                            .cursor_pointer()
                            .accessibility_label(format!("Provider {label}"))
                            .when(selected, |b| b.bg(rgb(palette().surface_hover)))
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.set_generation_channel(
                                        kind,
                                        engine,
                                        option.clone(),
                                        option_model.clone(),
                                    );
                                    this.persist();
                                    cx.notify();
                                });
                                if let Some(popover) = popover.clone().upgrade() {
                                    popover.update(cx, |p, cx| p.dismiss(window, cx));
                                }
                            })
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_1()
                                            .truncate()
                                            .text_size(px(12.))
                                            .child(label.clone()),
                                    )
                                    .when(selected, |r| {
                                        r.child(Icon::new(IconName::Check).small())
                                    }),
                            ),
                    );
                }
                list
            })
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{
        ConduitApp, GenerationKind, ModelChannel, ModelProvider, Provider, ProviderSource, Store,
        Workspace,
    };
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn generation_routes_are_independent_of_sessions_and_each_other(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut session = crate::app::tests::session();
                session.channel = ModelChannel::Provider("chat-route".into());
                session.model = Some("chat-model".into());
                let provider = |id: &str, engine, model: &str| ModelProvider {
                    id: id.into(),
                    name: id.into(),
                    engine,
                    base_url: format!("https://{id}.example/v1"),
                    api_key_env: String::new(),
                    models: vec![model.into()],
                    source: ProviderSource::Local,
                    model_metadata: vec![],
                };
                let store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir().join("conduit-generation-routes"),
                        sessions: vec![session],
                    }],
                    providers: vec![
                        provider("chat-route", Provider::Claude, "chat-model"),
                        provider("naming-route", Provider::Claude, "title-model"),
                        provider("git-route", Provider::Codex, "commit-model"),
                    ],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                let (engine, model, route) = app
                    .generation_request(GenerationKind::Naming, Some(1))
                    .unwrap();
                assert_eq!(engine, Provider::Claude);
                assert_eq!(model.as_deref(), Some("chat-model"));
                assert_eq!(
                    route.env["ANTHROPIC_BASE_URL"],
                    "https://chat-route.example/v1"
                );
                app.set_generation_model(GenerationKind::Naming, Some("fixed-title".into()));
                assert_eq!(app.store.ui.naming_provider, Some(Provider::Claude));
                assert_eq!(
                    app.store.ui.naming_channel,
                    Some(ModelChannel::Provider("chat-route".into()))
                );
                assert!(app.generation_uses_provider("chat-route"));
                app.store.ui.naming_provider = None;
                app.store.ui.naming_channel = None;
                app.store.ui.naming_model = None;
                // Pinning even the same engine must not inherit the chat's custom route/model.
                app.store.ui.naming_provider = Some(Provider::Claude);
                let (_, model, route) = app
                    .generation_request(GenerationKind::Naming, Some(1))
                    .unwrap();
                assert_eq!(model, None);
                assert!(route.env.is_empty());
                app.set_generation_channel(
                    GenerationKind::Naming,
                    Provider::Claude,
                    Some(ModelChannel::Provider("naming-route".into())),
                    None,
                );
                app.set_generation_channel(
                    GenerationKind::Git,
                    Provider::Codex,
                    Some(ModelChannel::Provider("git-route".into())),
                    Some("fixed-commit-model".into()),
                );
                let (_, model, route) = app
                    .generation_request(GenerationKind::Naming, Some(1))
                    .unwrap();
                assert_eq!(model, None);
                assert_eq!(route.default_model.as_deref(), Some("title-model"));
                assert_eq!(
                    route.env["ANTHROPIC_BASE_URL"],
                    "https://naming-route.example/v1"
                );
                let (engine, model, route) = app
                    .generation_request(GenerationKind::Git, Some(1))
                    .unwrap();
                assert_eq!(engine, Provider::Codex);
                assert_eq!(model.as_deref(), Some("fixed-commit-model"));
                assert_eq!(
                    route.config["model_providers"]["conduit"]["base_url"],
                    "https://git-route.example/v1"
                );
                assert_eq!(
                    app.generation_catalog(GenerationKind::Naming).models[0].id,
                    "title-model"
                );
                assert_eq!(
                    app.generation_catalog(GenerationKind::Git).models[0].id,
                    "commit-model"
                );
                assert_eq!(app.session(1).unwrap().model.as_deref(), Some("chat-model"));
                assert_eq!(
                    app.session(1).unwrap().channel,
                    ModelChannel::Provider("chat-route".into())
                );
                app.selected_session = None;
                assert!(app.generation_request(GenerationKind::Naming, None).is_ok());
                app.store.providers.retain(|p| p.id != "naming-route");
                assert!(
                    app.generation_request(GenerationKind::Naming, Some(1))
                        .is_err(),
                    "missing provider must not silently use chat credentials"
                );
                // Legacy CCS channels still resolve through the same registry.
                app.store.providers.push(ModelProvider {
                    id: ModelProvider::ccs_id(Provider::Claude, "legacy"),
                    ..provider("legacy", Provider::Claude, "legacy-model")
                });
                app.set_generation_channel(
                    GenerationKind::Naming,
                    Provider::Claude,
                    Some(ModelChannel::Ccs("legacy".into())),
                    None,
                );
                assert_eq!(
                    app.generation_request(GenerationKind::Naming, None)
                        .unwrap()
                        .2
                        .default_model
                        .as_deref(),
                    Some("legacy-model")
                );
                app.set_generation_channel(
                    GenerationKind::Git,
                    Provider::OpenCode,
                    Some(ModelChannel::Cli),
                    Some("native/glm-5".into()),
                );
                let (engine, model, route) = app
                    .generation_request(GenerationKind::Git, Some(1))
                    .unwrap();
                assert_eq!(engine, Provider::OpenCode);
                assert_eq!(model.as_deref(), Some("native/glm-5"));
                assert!(route.config.is_empty() && route.env.is_empty());
                assert!(
                    !app.generation_model_defaults(GenerationKind::Git).1,
                    "native provider cannot be cleared by model default"
                );
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }
}
