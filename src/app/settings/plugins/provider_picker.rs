use super::*;
use gpui_kit::component::searchable_list::{SearchableGroup, SearchableListDelegate};

#[derive(Clone, PartialEq)]
pub(super) struct ProviderChoice {
    pub id: String,
    pub models: Vec<String>,
    engine: Provider,
    name: String,
    source: String,
    detail: String,
}

impl SearchableListItem for ProviderChoice {
    type Value = String;

    fn title(&self) -> SharedString {
        format!("{} · {} · {}", self.name, self.engine.label(), self.detail).into()
    }

    fn value(&self) -> &String {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let text = self.title().to_lowercase();
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let id = self.id.clone();
        h_flex()
            .id(SharedString::from(format!(
                "encourager-provider-option-{id}"
            )))
            .debug_selector(move || format!("encourager-provider-option-{id}"))
            .aria_label(self.title())
            .gap_2()
            .child(self.name.clone())
            .child(caption(self.detail.clone()))
    }
}

pub(super) fn choices(providers: &[ModelProvider]) -> Vec<ProviderChoice> {
    let mut seen = HashSet::new();
    let mut choices: Vec<_> = providers
        .iter()
        .filter_map(|provider| {
            let models = crate::app::encourager::encourager_model_ids(provider);
            if models.is_empty() || !seen.insert(provider.id.clone()) {
                return None;
            }
            let source = match &provider.source {
                ProviderSource::Local => conduit::i18n::text("settings.services.local"),
                ProviderSource::Ccs { .. } => "cc-switch".into(),
            };
            Some(ProviderChoice {
                id: provider.id.clone(),
                models,
                engine: provider.engine,
                name: provider.name.clone(),
                detail: source.clone(),
                source,
            })
        })
        .collect();
    // Names are presentation only: different engines/accounts must keep their IDs.
    let mut names = HashMap::new();
    for choice in &choices {
        *names
            .entry((
                choice.engine,
                choice.name.to_lowercase(),
                choice.source.clone(),
            ))
            .or_insert(0) += 1;
    }
    for choice in &mut choices {
        if names[&(
            choice.engine,
            choice.name.to_lowercase(),
            choice.source.clone(),
        )] > 1
        {
            choice.detail = format!("{} · {}", choice.source, choice.id);
        }
    }
    choices.sort_by_cached_key(|choice| choice.name.to_lowercase());
    choices
}

type ProviderItems = SearchableVec<SearchableGroup<ProviderChoice>>;

fn grouped(choices: &[ProviderChoice]) -> ProviderItems {
    SearchableVec::new(
        Provider::ALL
            .into_iter()
            .filter_map(|engine| {
                let items: Vec<_> = choices
                    .iter()
                    .filter(|choice| choice.engine == engine)
                    .cloned()
                    .collect();
                (!items.is_empty()).then(|| SearchableGroup::new(engine.label()).items(items))
            })
            .collect::<Vec<_>>(),
    )
}

struct PickerState {
    select: Entity<SelectState<ProviderItems>>,
    choices: Vec<ProviderChoice>,
    _subscription: Subscription,
}

impl PickerState {
    fn new(
        app: &Entity<ConduitApp>,
        choices: Vec<ProviderChoice>,
        selected: Option<&String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let items = grouped(&choices);
        let index = selected.and_then(|id| items.position(id));
        let select = cx.new(|cx| SelectState::new(items, index, window, cx).searchable(true));
        let app = app.downgrade();
        let subscription = cx.subscribe(&select, move |_, _, event, cx| {
            let SelectEvent::Confirm(Some(id)) = event else {
                return;
            };
            let _ = app.update(cx, |app, cx| {
                // Also restore the displayed selection when a stale event is rejected.
                cx.notify();
                if !app.writable || app.closing {
                    return;
                }
                // Resolve again at confirmation in case a configuration refresh removed it.
                let Some(provider) = app
                    .store
                    .providers
                    .iter()
                    .find(|provider| &provider.id == id)
                else {
                    return;
                };
                let models = crate::app::encourager::encourager_model_ids(provider);
                if models.is_empty() {
                    return;
                }
                let settings = &app.store.ui.encourager;
                let model = settings
                    .model
                    .as_ref()
                    .filter(|model| {
                        settings.provider_id.as_ref() == Some(id) && models.contains(model)
                    })
                    .cloned()
                    .or_else(|| models.first().cloned());
                app.reset_encourager_output();
                app.store.ui.encourager.provider_id = Some(id.clone());
                app.store.ui.encourager.model = model;
                app.persist();
            });
        });
        Self {
            select,
            choices,
            _subscription: subscription,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct ProviderPicker {
    pub app: Entity<ConduitApp>,
    pub choices: Vec<ProviderChoice>,
    pub selected: Option<String>,
    pub disabled: bool,
}

impl RenderOnce for ProviderPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ("encourager-provider-state", self.app.entity_id()),
            cx,
            |window, cx| {
                PickerState::new(
                    &self.app,
                    self.choices.clone(),
                    self.selected.as_ref(),
                    window,
                    cx,
                )
            },
        );
        let empty = self.choices.is_empty();
        let select = state.update(cx, |state, cx| {
            let selected = self
                .selected
                .as_ref()
                .filter(|id| self.choices.iter().any(|choice| &choice.id == *id));
            let changed = state.choices != self.choices;
            if changed || state.select.read(cx).selected_value() != selected {
                state.select.update(cx, |select, cx| {
                    if changed {
                        select.set_items(grouped(&self.choices), window, cx);
                    }
                    // Rebase both the query and cursor when the catalog changes,
                    // including when there is no longer a selected provider.
                    select.set_selected_value(
                        self.selected.as_ref().unwrap_or(&String::new()),
                        window,
                        cx,
                    );
                    if selected.is_none() {
                        select.set_selected_index(None, window, cx);
                    }
                });
                state.choices = self.choices;
            }
            state.select.clone()
        });
        div()
            .debug_selector(|| "encourager-provider".into())
            .w_full()
            .child(
                Select::new(&select)
                    .id("encourager-provider-select")
                    .settings_control()
                    .w_full()
                    .disabled(self.disabled || empty)
                    .accessibility_label(conduit::i18n::text_static("encourager.setting.service"))
                    .placeholder(conduit::i18n::text_static(if empty {
                        "encourager.setting.no_service"
                    } else {
                        "encourager.no_service"
                    }))
                    .search_placeholder(conduit::i18n::text_static(
                        "encourager.setting.search_services",
                    ))
                    .menu_max_h(px(280.))
                    .empty(|_, _| {
                        caption(conduit::i18n::text_static(
                            "encourager.setting.no_matching_services",
                        ))
                        .debug_selector(|| "encourager-provider-empty".into())
                        .p_3()
                    }),
            )
    }
}
