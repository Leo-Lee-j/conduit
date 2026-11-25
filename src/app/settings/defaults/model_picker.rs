use super::*;
use gpui_kit::component::searchable_list::SearchableListDelegate;

#[derive(Clone, PartialEq)]
pub(super) struct ModelChoice {
    pub id: Option<String>,
    pub label: String,
}

impl SearchableListItem for ModelChoice {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn matches(&self, query: &str) -> bool {
        let text =
            format!("{} {}", self.label, self.id.as_deref().unwrap_or_default()).to_lowercase();
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }

    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        v_flex()
            .min_w_0()
            .gap_1()
            .child(div().truncate().child(self.label.clone()))
            .when_some(
                self.id.as_ref().filter(|id| **id != self.label),
                |row, id| {
                    row.child(
                        div()
                            .truncate()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(id.clone()),
                    )
                },
            )
    }
}

struct PickerState {
    select: Entity<SelectState<SearchableVec<ModelChoice>>>,
    choices: Vec<ModelChoice>,
    _subscription: Subscription,
}

impl PickerState {
    fn new(picker: &DefaultModelPicker, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let items = SearchableVec::new(picker.choices.clone());
        let index = items.position(&picker.selected);
        let select = cx.new(|cx| SelectState::new(items, index, window, cx).searchable(true));
        let app = picker.app.downgrade();
        let engine = picker.engine;
        let channel = picker.channel.clone();
        let subscription = cx.subscribe_in(&select, window, move |_, _, event, window, cx| {
            let SelectEvent::Confirm(Some(model)) = event else {
                return;
            };
            let _ = app.update(cx, |app, cx| {
                cx.notify();
                if app.connection_settings.engine != engine
                    || app.default_selection(engine).channel != channel
                    || !app.writable
                    || app.closing
                    || app.connection_settings.saving
                {
                    return;
                }
                let channel = default_channel_for_display(engine, &channel, &app.store.providers);
                app.set_service_default(engine, channel, model.clone(), window, cx);
            });
        });
        Self {
            select,
            choices: picker.choices.clone(),
            _subscription: subscription,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct DefaultModelPicker {
    pub app: Entity<ConduitApp>,
    pub engine: Provider,
    pub channel: ModelChannel,
    pub choices: Vec<ModelChoice>,
    pub selected: Option<String>,
    pub disabled: bool,
}

impl RenderOnce for DefaultModelPicker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key: SharedString = format!(
            "settings-default-model-{:?}-{:?}-{:?}",
            self.app.entity_id(),
            self.engine,
            self.channel
        )
        .into();
        let state =
            window.use_keyed_state(key, cx, |window, cx| PickerState::new(&self, window, cx));
        let select = state.update(cx, |state, cx| {
            if state.choices != self.choices
                || state.select.read(cx).selected_value() != Some(&self.selected)
            {
                state.select.update(cx, |select, cx| {
                    if state.choices != self.choices {
                        select.set_items(SearchableVec::new(self.choices.clone()), window, cx);
                    }
                    select.set_selected_value(&self.selected, window, cx);
                });
                state.choices = self.choices;
            }
            state.select.clone()
        });
        let engine = self.engine as usize;
        div()
            .debug_selector(move || format!("default-model-{engine}"))
            .w(rems(14.))
            .max_w_full()
            .child(
                Select::new(&select)
                    .id(("default-model", engine))
                    .small()
                    .w_full()
                    .disabled(self.disabled)
                    .accessibility_label(conduit::i18n::text_static(
                        "settings.sessions.default_model",
                    ))
                    .search_placeholder(conduit::i18n::text_static(
                        "model_picker.search_model_names_or_ids",
                    ))
                    .menu_max_h(px(280.))
                    .empty(|_, _| {
                        div()
                            .p_3()
                            .text_size(ui_text_size(12.))
                            .child(conduit::i18n::text_static(
                                "model_picker.no_matching_models",
                            ))
                    }),
            )
    }
}
