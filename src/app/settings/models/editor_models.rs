use super::*;
use crate::models::ModelOption;
use crate::providers::claude::{ClaudeModels, Mapping, Role, split_one_m, with_one_m};
use gpui_kit::component::{
    checkbox::Checkbox,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
};

pub(super) struct ModelsChanged;
pub(super) struct FetchModels;

struct ModelRow {
    id: usize,
    name: Entity<InputState>,
    model: Entity<InputState>,
    picker: Entity<ComboboxState<SearchableVec<SharedString>>>,
    one_m: bool,
    metadata: Option<ModelOption>,
    _subscriptions: Vec<Subscription>,
}

impl ModelRow {
    fn mapping(&self, cx: &App, claude: bool) -> Mapping {
        let value = self.model.read(cx).value();
        Mapping {
            model: if claude {
                with_one_m(&value, self.one_m || split_one_m(&value).1)
            } else {
                value.trim().into()
            },
            display_name: self.name.read(cx).value().trim().into(),
        }
    }

    fn set_model(&mut self, model: &str, window: &mut Window, cx: &mut App) {
        let old = self.model.read(cx).value();
        let (base, one_m) = if self.id < 6 {
            split_one_m(model)
        } else {
            (model.trim(), false)
        };
        let name = self.name.read(cx).value();
        if name.is_empty() || name == old {
            self.name
                .update(cx, |input, cx| input.set_value(base, window, cx));
        }
        self.one_m |= one_m;
        self.model
            .update(cx, |input, cx| input.set_value(base, window, cx));
    }
}

#[derive(Clone, PartialEq)]
struct Snapshot {
    rows: Vec<Mapping>,
    roles: Vec<Mapping>,
    fallback: Mapping,
}

pub(super) struct ModelDraft {
    pub models: Vec<String>,
    pub metadata: Vec<ModelOption>,
    pub claude: Option<ClaudeModels>,
}

pub(super) struct ProviderModels {
    pub engine: Provider,
    pub disabled: bool,
    rows: Vec<ModelRow>,
    roles: Vec<ModelRow>,
    fallback: ModelRow,
    catalog: Vec<String>,
    next_id: usize,
    initial: Option<Snapshot>,
}

impl gpui_kit::EventEmitter<ModelsChanged> for ProviderModels {}
impl gpui_kit::EventEmitter<FetchModels> for ProviderModels {}

impl ProviderModels {
    pub fn new(
        provider: Option<&ModelProvider>,
        engine: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalog = provider.map(|p| p.models.clone()).unwrap_or_default();
        let mapping = provider
            .and_then(|p| p.claude_models.clone())
            .unwrap_or_else(|| ClaudeModels {
                default_model: if engine == Provider::Claude {
                    catalog.first().cloned().unwrap_or_default()
                } else {
                    String::new()
                },
                ..Default::default()
            });
        let fallback = Self::new_row(
            0,
            &Mapping {
                model: mapping.default_model.clone(),
                ..Default::default()
            },
            &catalog,
            true,
            window,
            cx,
        );
        let roles = Role::ALL
            .into_iter()
            .enumerate()
            .map(|(i, role)| {
                Self::new_row(
                    i + 1,
                    mapping.roles.get(&role).unwrap_or(&Mapping::default()),
                    &catalog,
                    true,
                    window,
                    cx,
                )
            })
            .collect();
        let mut this = Self {
            engine,
            disabled: false,
            rows: vec![],
            roles,
            fallback,
            catalog,
            next_id: 6,
            initial: None,
        };
        if let Some(provider) = provider {
            for id in &provider.models {
                let metadata = provider
                    .model_metadata
                    .iter()
                    .find(|m| m.id == *id)
                    .cloned();
                let mapping = Mapping {
                    model: id.clone(),
                    display_name: metadata
                        .as_ref()
                        .map(|m| m.label.clone())
                        .unwrap_or_default(),
                };
                let mut row =
                    Self::new_row(this.next_id, &mapping, &this.catalog, false, window, cx);
                row.metadata = metadata;
                this.next_id += 1;
                this.rows.push(row);
            }
        }
        if this.rows.is_empty() && engine == Provider::Codex {
            this.add_row(window, cx);
        }
        this.initial = Some(this.snapshot(cx));
        this
    }

    fn new_row(
        id: usize,
        mapping: &Mapping,
        catalog: &[String],
        claude: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ModelRow {
        let (base, one_m) = if claude {
            split_one_m(&mapping.model)
        } else {
            (mapping.model.as_str(), false)
        };
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(mapping.display_name.clone())
                .placeholder(conduit::i18n::text_static(
                    "settings.models.display_name_optional",
                ))
        });
        let model =
            cx.new(|cx| {
                InputState::new(window, cx).default_value(base).placeholder(
                    conduit::i18n::text_static("settings.models.type_or_select_model"),
                )
            });
        let picker = cx.new(|cx| {
            ComboboxState::new(Self::choices(catalog), vec![], window, cx).searchable(true)
        });
        let picker_reset = picker.clone();
        let mut subscriptions = vec![
            cx.subscribe(&model, move |_, _, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                picker_reset.update(cx, |picker, cx| picker.clear_selection(cx));
                cx.emit(ModelsChanged);
                cx.notify();
            }),
            cx.subscribe(&name, |_, _, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Change) {
                    return;
                }
                cx.emit(ModelsChanged);
                cx.notify();
            }),
        ];
        let view = cx.entity();
        subscriptions.push(window.subscribe(
            &picker,
            cx,
            move |_, event: &ComboboxEvent<SearchableVec<SharedString>>, window, cx| {
                if let ComboboxEvent::Change(values) = event
                    && let Some(value) = values.first()
                {
                    view.update(cx, |this, cx| {
                        if this.disabled {
                            return;
                        }
                        if let Some(row) = this.row_mut(id) {
                            row.set_model(value, window, cx);
                        }
                        cx.emit(ModelsChanged);
                        cx.notify();
                    });
                }
            },
        ));
        ModelRow {
            id,
            name,
            model,
            picker,
            one_m,
            metadata: None,
            _subscriptions: subscriptions,
        }
    }

    fn choices(catalog: &[String]) -> SearchableVec<SharedString> {
        SearchableVec::new(
            catalog
                .iter()
                .cloned()
                .map(SharedString::from)
                .collect::<Vec<_>>(),
        )
    }

    fn row_mut(&mut self, id: usize) -> Option<&mut ModelRow> {
        self.rows
            .iter_mut()
            .chain(&mut self.roles)
            .chain(std::iter::once(&mut self.fallback))
            .find(|row| row.id == id)
    }

    fn snapshot(&self, cx: &App) -> Snapshot {
        Snapshot {
            rows: self.rows.iter().map(|row| row.mapping(cx, false)).collect(),
            roles: self.roles.iter().map(|row| row.mapping(cx, true)).collect(),
            fallback: self.fallback.mapping(cx, true),
        }
    }

    pub fn is_dirty(&self, cx: &App) -> bool {
        self.initial
            .as_ref()
            .is_some_and(|initial| *initial != self.snapshot(cx))
    }

    pub fn set_catalog(
        &mut self,
        catalog: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalog = catalog;
        for row in self
            .rows
            .iter()
            .chain(&self.roles)
            .chain(std::iter::once(&self.fallback))
        {
            row.picker.update(cx, |picker, cx| {
                picker.set_items(Self::choices(&self.catalog), window, cx)
            });
        }
        cx.notify();
    }

    fn add_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = Self::new_row(
            self.next_id,
            &Mapping::default(),
            &self.catalog,
            false,
            window,
            cx,
        );
        self.next_id += 1;
        self.rows.push(row);
        cx.emit(ModelsChanged);
        cx.notify();
    }

    fn quick_set(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self
            .roles
            .iter()
            .chain(std::iter::once(&self.fallback))
            .map(|row| row.mapping(cx, true))
            .find(|m| !m.model.is_empty());
        if let Some(source) = source {
            for row in &mut self.roles {
                row.one_m = split_one_m(&source.model).1;
                row.model.update(cx, |input, cx| {
                    input.set_value(split_one_m(&source.model).0, window, cx)
                });
                row.name.update(cx, |input, cx| {
                    input.set_value(split_one_m(&source.model).0, window, cx)
                });
            }
            if self.fallback.model.read(cx).value().trim().is_empty() {
                self.fallback.set_model(&source.model, window, cx);
            }
            cx.emit(ModelsChanged);
            cx.notify();
        }
    }

    pub fn draft(&self, cx: &App) -> anyhow::Result<ModelDraft> {
        let mut models = vec![];
        let mut metadata = vec![];
        for row in &self.rows {
            let mapping = row.mapping(cx, false);
            if mapping.model.is_empty() && mapping.display_name.is_empty() {
                continue;
            }
            anyhow::ensure!(
                !mapping.model.is_empty()
                    && mapping.model.len() <= 512
                    && !mapping
                        .model
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control())
                    && !mapping.display_name.chars().any(char::is_control),
                conduit::i18n::text("settings.models.invalid_mapping")
            );
            anyhow::ensure!(
                !models.contains(&mapping.model),
                conduit::i18n::text("settings.models.duplicate_model")
            );
            let previous = row.metadata.as_ref().filter(|m| m.id == mapping.model);
            metadata.push(ModelOption {
                id: mapping.model.clone(),
                label: if mapping.display_name.is_empty() {
                    mapping.model.clone()
                } else {
                    mapping.display_name
                },
                note: previous.map(|m| m.note.clone()).unwrap_or_default(),
                context_window: previous.and_then(|m| m.context_window),
            });
            models.push(mapping.model);
        }
        let claude = if self.engine == Provider::Claude {
            let mapping = ClaudeModels {
                default_model: self.fallback.mapping(cx, true).model,
                roles: Role::ALL
                    .into_iter()
                    .zip(&self.roles)
                    .map(|(role, row)| {
                        let mut mapping = row.mapping(cx, true);
                        if role == Role::Subagent {
                            mapping.display_name.clear();
                        }
                        (role, mapping)
                    })
                    .filter(|(_, m)| !m.model.is_empty() || !m.display_name.is_empty())
                    .collect(),
            };
            mapping.validate()?;
            if !mapping.default_model.is_empty() && !models.contains(&mapping.default_model) {
                models.insert(0, mapping.default_model.clone());
            }
            Some(mapping)
        } else {
            anyhow::ensure!(
                !models.is_empty(),
                conduit::i18n::text("providers.enter_at_least_one_model_id")
            );
            None
        };
        Ok(ModelDraft {
            models,
            metadata,
            claude,
        })
    }

    fn model_input(&self, row: &ModelRow, label: String) -> impl IntoElement {
        let id = row.id;
        h_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .debug_selector(move || format!("provider-model-input-{id}"))
                    .child(
                        Input::new(&row.model)
                            .disabled(self.disabled)
                            .aria_label(label),
                    ),
            )
            .child(
                div()
                    .w(px(32.))
                    .flex_shrink_0()
                    .debug_selector(move || format!("provider-model-picker-{id}"))
                    .child(
                        Combobox::new(&row.picker)
                            .w(px(32.))
                            .px_1()
                            .disabled(self.disabled)
                            .menu_width(px(300.))
                            .menu_max_h(px(260.))
                            .search_placeholder(conduit::i18n::text_static(
                                "settings.models.search_models",
                            ))
                            .render_trigger(|_, _, _| Icon::new(IconName::ChevronDown).small())
                            .empty(|_, _| {
                                div()
                                    .p_3()
                                    .text_size(px(12.))
                                    .child(conduit::i18n::text_static(
                                        "settings.models.fetch_or_type_model",
                                    ))
                            }),
                    ),
            )
    }

    fn one_m(&self, row: &ModelRow, cx: &Context<Self>) -> impl IntoElement {
        let id = row.id;
        Checkbox::new(("provider-model-one-m", id))
            .label("1M")
            .checked(row.one_m || split_one_m(&row.model.read(cx).value()).1)
            .disabled(self.disabled)
            .accessibility_label(conduit::i18n::text_static("settings.models.supports_one_m"))
            .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                if let Some(row) = this.row_mut(id) {
                    row.one_m = *checked;
                    let value = row.model.read(cx).value();
                    row.model.update(cx, |input, cx| {
                        input.set_value(split_one_m(&value).0, window, cx)
                    });
                }
                cx.emit(ModelsChanged);
                cx.notify();
            }))
    }

    fn mapping_table(&self, narrow: bool, cx: &Context<Self>) -> impl IntoElement {
        let mut table = v_flex().w_full().gap_3();
        if !narrow {
            table = table.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(
                        div()
                            .w(px(82.))
                            .child(conduit::i18n::text_static("settings.models.model_role")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(conduit::i18n::text_static("settings.models.display_name")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(conduit::i18n::text_static("settings.models.request_model")),
                    )
                    .child(
                        div()
                            .w(px(84.))
                            .child(conduit::i18n::text_static("settings.models.supports_one_m")),
                    ),
            );
        }
        for (role, row) in Role::ALL.into_iter().zip(&self.roles) {
            let name = if role == Role::Subagent {
                div()
                    .min_w_0()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(
                        "settings.models.subagent_no_menu",
                    ))
                    .into_any_element()
            } else {
                Input::new(&row.name)
                    .disabled(self.disabled)
                    .aria_label(format!(
                        "{} {}",
                        role.label(),
                        conduit::i18n::text_static("settings.models.display_name")
                    ))
                    .into_any_element()
            };
            table = table.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(narrow, |row| row.flex_col().items_start())
                    .debug_selector(move || format!("provider-role-{}", role.alias()))
                    .child(
                        div()
                            .w(px(82.))
                            .flex_shrink_0()
                            .rounded_md()
                            .bg(rgb(palette().surface_strong))
                            .px_3()
                            .py_2()
                            .text_size(ui_text_size(12.))
                            .font_medium()
                            .child(role.label()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .when(narrow, |d| d.w_full())
                            .child(name),
                    )
                    .child(div().flex_1().min_w_0().when(narrow, |d| d.w_full()).child(
                        self.model_input(
                            row,
                            format!(
                                "{} {}",
                                role.label(),
                                conduit::i18n::text_static("settings.models.request_model")
                            ),
                        ),
                    ))
                    .child(div().w(px(84.)).flex_shrink_0().child(self.one_m(row, cx))),
            );
        }
        table.child(
            v_flex()
                .gap_2()
                .border_t_1()
                .border_color(rgb(palette().border))
                .pt_3()
                .child(provider_field_label(
                    conduit::i18n::text_static("settings.models.default_fallback"),
                    false,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().min_w_0().child(self.model_input(
                            &self.fallback,
                            conduit::i18n::text("settings.models.default_fallback"),
                        )))
                        .child(div().w(px(84.)).child(self.one_m(&self.fallback, cx))),
                )
                .child(
                    div()
                        .text_size(ui_text_size(11.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("settings.models.mapping_hint")),
                ),
        )
    }
}

impl Render for ProviderModels {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let claude = self.engine == Provider::Claude;
        let narrow = window.viewport_size().width < px(740.);
        let quick_enabled = self
            .roles
            .iter()
            .chain(std::iter::once(&self.fallback))
            .any(|row| !row.model.read(cx).value().trim().is_empty());
        let mut body = v_flex().w_full().gap_3().child(
            h_flex()
                .w_full()
                .justify_between()
                .flex_wrap()
                .gap_2()
                .child(provider_field_label(
                    conduit::i18n::text_static(if claude {
                        "settings.models.model_mapping"
                    } else {
                        "settings.models.model_list"
                    }),
                    true,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .when(claude, |actions| {
                            actions.child(
                                Button::new("provider-model-quick-set")
                                    .small()
                                    .outline()
                                    .label(conduit::i18n::text_static("settings.models.quick_set"))
                                    .disabled(self.disabled || !quick_enabled)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.quick_set(window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("provider-discover")
                                .small()
                                .outline()
                                .icon(FullIconName::Download)
                                .label(conduit::i18n::text_static(if self.disabled {
                                    "settings.models.requesting"
                                } else {
                                    "settings.models.fetch_model_list"
                                }))
                                .disabled(self.disabled)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(FetchModels))),
                        ),
                ),
        );
        if claude {
            body = body.child(self.mapping_table(narrow, cx));
        }
        body = body.child(
            h_flex()
                .w_full()
                .justify_between()
                .gap_2()
                .when(claude, |row| {
                    row.child(provider_field_label(
                        conduit::i18n::text_static("settings.models.additional_models"),
                        false,
                    ))
                })
                .child(
                    Button::new("provider-model-add")
                        .small()
                        .ghost()
                        .icon(FullIconName::Plus)
                        .label(conduit::i18n::text_static("settings.models.add_model"))
                        .disabled(self.disabled)
                        .on_click(cx.listener(|this, _, window, cx| this.add_row(window, cx))),
                ),
        );
        if !self.rows.is_empty() && !narrow {
            body = body.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().muted))
                    .child(
                        div()
                            .w(px(82.))
                            .child(conduit::i18n::text_static("settings.models.model_role")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(conduit::i18n::text_static("settings.models.display_name")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(conduit::i18n::text_static("settings.models.request_model")),
                    )
                    .child(div().w(px(28.))),
            );
        }
        for (index, row) in self.rows.iter().enumerate() {
            let id = row.id;
            body = body.child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .when(narrow, |row| row.flex_col().items_start())
                    .debug_selector(move || format!("provider-model-row-{id}"))
                    .child(
                        Button::new(("provider-model-default", id))
                            .w(px(82.))
                            .small()
                            .ghost()
                            .label(if index == 0 && !claude {
                                conduit::i18n::text_static("model_picker.default_model").to_owned()
                            } else {
                                format!("#{}", index + 1)
                            })
                            .tooltip(conduit::i18n::text_static(
                                "settings.models.set_default_model",
                            ))
                            .disabled(self.disabled || claude || index == 0)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(index) = this.rows.iter().position(|r| r.id == id) {
                                    let row = this.rows.remove(index);
                                    this.rows.insert(0, row);
                                }
                                cx.emit(ModelsChanged);
                                cx.notify();
                            })),
                    )
                    .child(
                        div().flex_1().min_w_0().when(narrow, |d| d.w_full()).child(
                            Input::new(&row.name).disabled(self.disabled).aria_label(
                                conduit::i18n::text_static("settings.models.display_name"),
                            ),
                        ),
                    )
                    .child(div().flex_1().min_w_0().when(narrow, |d| d.w_full()).child(
                        self.model_input(row, conduit::i18n::text("settings.models.request_model")),
                    ))
                    .child(
                        Button::new(("provider-model-remove", id))
                            .small()
                            .ghost()
                            .icon(FullIconName::X)
                            .accessibility_label(conduit::i18n::text_static(
                                "settings.models.remove_model",
                            ))
                            .disabled(self.disabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.rows.retain(|r| r.id != id);
                                cx.emit(ModelsChanged);
                                cx.notify();
                            })),
                    ),
            );
        }
        body
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ModelProvider, Provider, ProviderModels, Role};
    use gpui_kit::{AppContext as _, Modifiers, MouseButton, TestAppContext, px, size};

    #[gpui_kit::test]
    fn model_rows_keep_metadata_and_catalog_refresh_preserves_drafts(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let provider: ModelProvider = serde_json::from_value(serde_json::json!({
                "id":"legacy", "name":"Legacy", "engine":"Codex", "base_url":"https://example.invalid", "api_key_env":"",
                "models":["first", "second"], "model_metadata":[{"id":"first","label":"Friendly","note":"keep note","context_window":64000}]
            })).unwrap();
            let editor = cx.new(|cx| ProviderModels::new(Some(&provider), Provider::Codex, window, cx));
            editor.update(cx, |editor, cx| {
                assert!(!editor.is_dirty(cx));
                editor.set_catalog(vec!["new-candidate".into()], window, cx);
                assert!(!editor.is_dirty(cx), "fetching candidates must not change configured models");
                let draft = editor.draft(cx).unwrap();
                assert_eq!(draft.models, ["first", "second"]);
                assert_eq!(draft.metadata[0].label, "Friendly");
                assert_eq!(draft.metadata[0].note, "keep note");
                assert_eq!(draft.metadata[0].context_window, Some(64000));
                editor.rows[0].name.update(cx, |input, cx| input.set_value("Renamed", window, cx));
                assert!(editor.is_dirty(cx));
                editor.rows.swap(0, 1);
                assert_eq!(editor.draft(cx).unwrap().models[0], "second");
                editor.add_row(window, cx);
                editor.rows.last_mut().unwrap().set_model("first", window, cx);
                assert!(editor.draft(cx).is_err(), "duplicate rows must not silently discard a display name");
            });
            gpui_kit::component::Root::new(editor, window, cx)
        });
    }

    #[gpui_kit::test]
    fn claude_mapping_quick_set_picker_and_reopen_preserve_runtime_values(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let editor = cx.new(|cx| ProviderModels::new(None, Provider::Claude, window, cx));
            editor.update(cx, |editor, cx| {
                assert!(!editor.is_dirty(cx));
                editor.fallback.set_model("custom[1M]", window, cx);
                editor.quick_set(window, cx);
                editor.roles[0]
                    .name
                    .update(cx, |input, cx| input.set_value("Everyday", window, cx));
                editor.roles[3].one_m = false;
                editor.set_catalog(vec!["custom".into(), "reasoner".into()], window, cx);
                let mapping = editor.draft(cx).unwrap().claude.unwrap();
                assert_eq!(mapping.roles[&Role::Sonnet].display_name, "Everyday");
                assert_eq!(mapping.roles[&Role::Haiku].model, "custom");
                assert_eq!(mapping.roles[&Role::Opus].model, "custom[1M]");
                assert!(mapping.roles[&Role::Subagent].display_name.is_empty());
            });
            *output.borrow_mut() = Some(editor.clone());
            gpui_kit::component::Root::new(editor, window, cx)
        });
        let editor = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(size(px(1000.), px(850.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("provider-role-subagent").is_some());
        assert!(cx.debug_bounds("provider-model-picker-1").is_some());
        let trigger = cx.debug_bounds("provider-model-picker-1").unwrap().center();
        cx.simulate_mouse_down(trigger, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(trigger, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
        cx.update(|window, cx| {
            let picker = editor.read(cx).roles[0].picker.clone();
            picker.update(cx, |picker, cx| picker.set_query("reasoner", window, cx));
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("down enter");
        cx.run_until_parked();
        cx.update(|window, cx| {
            let draft = editor.read(cx).draft(cx).unwrap();
            assert_eq!(
                draft.claude.as_ref().unwrap().roles[&Role::Sonnet].model,
                "reasoner[1M]"
            );
            assert_eq!(
                draft.claude.as_ref().unwrap().roles[&Role::Sonnet].display_name,
                "Everyday"
            );
            let provider = ModelProvider {
                id: "edited".into(),
                name: "Proxy".into(),
                engine: Provider::Claude,
                base_url: "https://example.invalid/v1".into(),
                api_key_env: String::new(),
                protected_api_key: None,
                models: draft.models,
                model_metadata: draft.metadata,
                claude_models: draft.claude,
                source: Default::default(),
            };
            let provider: ModelProvider =
                serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();
            let route = provider.route().unwrap();
            assert_eq!(route.env["ANTHROPIC_BASE_URL"], "https://example.invalid");
            assert_eq!(route.env[Role::Sonnet.env()], "reasoner[1M]");
            assert_eq!(route.env[Role::Subagent.env()], "custom[1M]");
            assert_eq!(route.default_model.as_deref(), Some("custom[1M]"));
            let reopened =
                cx.new(|cx| ProviderModels::new(Some(&provider), Provider::Claude, window, cx));
            assert!(!reopened.read(cx).is_dirty(cx));
            assert_eq!(
                reopened.read(cx).draft(cx).unwrap().claude,
                provider.claude_models
            );
        });
        cx.simulate_resize(size(px(520.), px(850.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        let row = cx.debug_bounds("provider-role-sonnet").unwrap();
        let input = cx.debug_bounds("provider-model-input-1").unwrap();
        assert!(input.left() >= row.left() && input.right() <= row.right());
    }
}
