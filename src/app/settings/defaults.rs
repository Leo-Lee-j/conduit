//! Persistent defaults apply only when constructing a new conversation.
use super::*;

impl ConduitApp {
    pub(in crate::app) fn default_selection(&self, engine: Provider) -> ComposerSelection {
        let mut selection = ComposerSelection::new(engine);
        if let Some(defaults) = self.store.session_defaults.get(&engine) {
            selection.channel = defaults.channel.clone();
            selection.model = defaults.model.clone();
        }
        selection
    }

    pub(super) fn settings_defaults_page(&self, disabled: bool, view: Entity<Self>) -> SettingPage {
        let groups = Provider::ALL
            .into_iter()
            .map(|engine| {
                let selection = self.default_selection(engine);
                let mut channels = vec![(ModelChannel::Cli, "CLI 原生配置".to_string())];
                channels.extend(
                    self.store
                        .providers
                        .iter()
                        .filter(|p| p.engine == engine)
                        .map(|p| (ModelChannel::Provider(p.id.clone()), p.name.clone())),
                );
                let label = channels
                    .iter()
                    .find(|(c, _)| *c == selection.channel)
                    .map(|(_, label)| label.clone())
                    .unwrap_or_else(|| "服务已不可用，请重新选择".into());
                let route_view = view.clone();
                let route = SettingField::render(move |_, _, _| {
                    let channels = channels.clone();
                    let view = route_view.clone();
                    Button::new(("default-channel", engine as usize))
                        .outline()
                        .small()
                        .label(label.clone())
                        .icon(FullIconName::ChevronDown)
                        .disabled(disabled)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (channel, label) in &channels {
                                let channel = channel.clone();
                                let view = view.clone();
                                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                    move |_, _, cx| {
                                        view.update(cx, |app, cx| {
                                            if !app.writable || app.closing {
                                                return;
                                            }
                                            app.store.session_defaults.insert(
                                                engine,
                                                crate::store::SessionDefaults {
                                                    channel: channel.clone(),
                                                    model: None,
                                                },
                                            );
                                            app.persist();
                                            cx.notify();
                                        });
                                    },
                                ));
                            }
                            menu
                        })
                });
                let mut models = vec![(None, "跟随服务默认模型".to_string())];
                let catalog = match &selection.channel {
                    ModelChannel::Provider(id) => self
                        .store
                        .providers
                        .iter()
                        .find(|p| &p.id == id)
                        .map(|p| p.catalog())
                        .unwrap_or_default(),
                    _ => self
                        .engine_catalog(engine)
                        .map(|c| c.models.clone())
                        .unwrap_or_default(),
                };
                models.extend(catalog.into_iter().map(|m| (Some(m.id), m.label)));
                let label = selection.model.unwrap_or_else(|| "跟随服务默认模型".into());
                let model_view = view.clone();
                let model = SettingField::render(move |_, _, _| {
                    let models = models.clone();
                    let view = model_view.clone();
                    Button::new(("default-model", engine as usize))
                        .outline()
                        .small()
                        .label(label.clone())
                        .icon(FullIconName::ChevronDown)
                        .disabled(disabled)
                        .dropdown_menu(move |mut menu, _, _| {
                            for (model, label) in &models {
                                let model = model.clone();
                                let view = view.clone();
                                menu = menu.item(PopupMenuItem::new(label.clone()).on_click(
                                    move |_, _, cx| {
                                        view.update(cx, |app, cx| {
                                            if !app.writable || app.closing {
                                                return;
                                            }
                                            app.store
                                                .session_defaults
                                                .entry(engine)
                                                .or_default()
                                                .model = model.clone();
                                            app.persist();
                                            cx.notify();
                                        });
                                    },
                                ));
                            }
                            menu
                        })
                });
                settings_section(engine.label()).items(vec![
                    SettingItem::new("模型服务", route),
                    SettingItem::new("模型", model),
                ])
            })
            .collect::<Vec<_>>();
        settings_page("新会话默认", FullIconName::Settings).groups(groups)
    }

    pub(super) fn provider_references(&self, id: &str) -> Vec<String> {
        let channel = ModelChannel::Provider(id.into());
        let mut references = vec![];
        for (engine, defaults) in &self.store.session_defaults {
            if defaults.channel == channel {
                references.push(format!("{} 新会话默认", engine.label()));
            }
        }
        if self.generation_uses_provider(id)
            && self.store.ui.naming_channel.as_ref() == Some(&channel)
        {
            references.push("自动命名".into());
        }
        if self.store.git.generation_channel.as_ref() == Some(&channel) {
            references.push("Git 提交说明".into());
        }
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
    use super::{ConduitApp, ModelChannel, Provider, Store, SystemTime, UNIX_EPOCH};
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn new_session_defaults_persist_and_explicit_drafts_win_without_changing_history(
        cx: &mut TestAppContext,
    ) {
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
