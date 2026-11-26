//! Compact appearance properties. Filters are transient; values remain in UiSettings.
use super::*;
use crate::store::UiSettings;
use gpui_kit::component::setting::RenderOptions;

mod fields;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum AppearanceGroup {
    Theme,
    Interface,
    Chat,
    Code,
    Background,
}

impl AppearanceGroup {
    const ALL: [Self; 5] = [
        Self::Theme,
        Self::Interface,
        Self::Chat,
        Self::Code,
        Self::Background,
    ];

    fn label(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Theme => "settings.appearance.theme",
            Self::Interface => "settings.appearance.interface_typography",
            Self::Chat => "settings.appearance.chat_text",
            Self::Code => "settings.appearance.code_blocks",
            Self::Background => "wallpaper.title",
        })
    }

    // Keep the existing global-search/direct-entry identities.
    fn legacy_group(self) -> usize {
        match self {
            Self::Theme => 0,
            Self::Interface | Self::Chat | Self::Code => 1,
            Self::Background => 2,
        }
    }

    fn scope(self) -> &'static str {
        conduit::i18n::text_static(match self {
            Self::Interface => "settings.appearance.interface_group_scope",
            Self::Chat => "settings.appearance.chat_typography_scope",
            Self::Code => "settings.appearance.code_typography_scope",
            _ => "settings.appearance.application_scope",
        })
    }
}

struct Property {
    id: &'static str,
    title: &'static str,
    group: AppearanceGroup,
    changed: bool,
    item: SettingItem,
}

impl Property {
    fn matches(
        &self,
        query: &str,
        modified_only: bool,
        group: Option<AppearanceGroup>,
        legacy: Option<usize>,
    ) -> bool {
        (!modified_only || self.changed)
            && group.is_none_or(|group| group == self.group)
            && legacy.is_none_or(|group| group == self.group.legacy_group())
            && navigation::matches_search(
                &format!(
                    "{} {} {}",
                    self.id.replace('_', " "),
                    self.title,
                    self.group.label()
                ),
                &query.replace('_', " "),
            )
    }

    fn exact_match(&self, query: &str) -> bool {
        !query.trim().is_empty()
            && (self.title.eq_ignore_ascii_case(query.trim())
                || self.id.eq_ignore_ascii_case(query.trim()))
    }
}

#[derive(IntoElement)]
struct PropertyList {
    properties: Vec<Property>,
    changed_groups: [bool; 5],
    view: Entity<ConduitApp>,
    stacked: bool,
}

impl RenderOnce for PropertyList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let disabled = !self.view.read(cx).writable || self.view.read(cx).closing;
        let query = self
            .view
            .read(cx)
            .settings_ui
            .appearance_filter
            .read(cx)
            .value();
        let mut groups = AppearanceGroup::ALL;
        groups.sort_by_key(|group| {
            !self
                .properties
                .iter()
                .any(|p| p.group == *group && p.exact_match(&query))
        });
        let mut list = v_flex().w_full().min_w_0().gap_3();
        for group in groups {
            let mut fields: Vec<_> = self
                .properties
                .iter()
                .filter(|p| p.group == group)
                .collect();
            if fields.is_empty() {
                continue;
            }
            fields.sort_by_key(|property| !property.exact_match(&query));
            let changed = self.changed_groups[group as usize];
            let reset = self.view.clone();
            let mut section = v_flex().w_full().min_w_0().child(
                h_flex()
                    .min_h(px(30.))
                    .gap_2()
                    .flex_wrap()
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .font_semibold()
                            .child(group.label()),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(("appearance-reset-group", group as usize))
                            .debug_selector(move || {
                                format!("appearance-reset-group-{}", group as usize)
                            })
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static(
                                "settings.appearance.reset_group",
                            ))
                            .accessibility_label(format!(
                                "{}: {}",
                                conduit::i18n::text_static("settings.appearance.reset_group"),
                                group.label()
                            ))
                            .disabled(disabled || !changed)
                            .on_click(move |_, window, cx| {
                                reset.update(cx, |app, cx| {
                                    app.reset_appearance_group(group, window, cx);
                                    if app.settings_ui.appearance_modified_only {
                                        app.settings_ui
                                            .search
                                            .read(cx)
                                            .focus_handle(cx)
                                            .focus(window, cx);
                                    }
                                })
                            }),
                    ),
            );
            for property in fields {
                let options = RenderOptions::new()
                    .with_item_ix(property_index(property.id))
                    .with_size(Size::Small)
                    .with_disabled(disabled)
                    .with_layout(if self.stacked {
                        Axis::Vertical
                    } else {
                        Axis::Horizontal
                    });
                section = section.child(
                    h_flex()
                        .id(property.id)
                        .debug_selector(move || format!("appearance-property-{}", property.id))
                        .w_full()
                        .min_w_0()
                        .min_h(ui_text_size(36.))
                        .py_1()
                        .gap_2()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .w(px(4.))
                                .flex_shrink_0()
                                .when(property.changed, |mark| {
                                    mark.h(px(4.)).rounded_full().bg(rgb(palette().focus))
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(property.item.clone().render_item(&options, window, cx)),
                        ),
                );
            }
            list = list.child(section);
        }
        list
    }
}

// Stable item identities prevent filtering from reusing another field's local state.
fn property_index(id: &str) -> usize {
    id.bytes().fold(0usize, |value, byte| {
        value.wrapping_mul(31).wrapping_add(byte as usize)
    })
}

impl ConduitApp {
    pub(in crate::app) fn reset_appearance_group(
        &mut self,
        group: AppearanceGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let defaults = UiSettings::default();
        match group {
            AppearanceGroup::Interface => {
                self.store.ui.icon_size = defaults.icon_size;
                super::super::super::theme::apply_icon_size(defaults.icon_size, cx);
                self.reset_ui_typography(window, cx);
            }
            AppearanceGroup::Chat => self.reset_chat_typography(window, cx),
            AppearanceGroup::Code => self.reset_code_typography(window, cx),
            AppearanceGroup::Theme => {
                self.store.ui.appearance = defaults.appearance;
                self.store.ui.light_theme = defaults.light_theme;
                self.store.ui.dark_theme = defaults.dark_theme;
                for (select, name) in [
                    (&self.light_theme_select, &self.store.ui.light_theme),
                    (&self.dark_theme_select, &self.store.ui.dark_theme),
                ] {
                    select.update(cx, |state, cx| {
                        state.set_selected_value(&SharedString::from(name.clone()), window, cx)
                    });
                }
                self.apply_appearance(window, cx);
                self.persist();
            }
            AppearanceGroup::Background => self.reset_background_preferences(window, cx),
        }
        cx.notify();
    }

    pub(in crate::app::settings) fn settings_appearance_content(
        &self,
        available_width: f32,
        view: Entity<Self>,
        cx: &App,
    ) -> SettingsContent {
        let scale = (self.store.ui.ui_font_size / DEFAULT_UI_FONT_SIZE).max(1.);
        let wide_preview =
            self.settings_ui.appearance_preview_open && available_width >= 760. * scale;
        let preview_width = 280. * scale;
        let property_width = available_width - if wide_preview { preview_width } else { 0. };
        let query = self.settings_ui.appearance_filter.read(cx).value();
        let all_properties = self.appearance_properties(view.clone());
        let changed_groups = AppearanceGroup::ALL.map(|group| {
            all_properties
                .iter()
                .any(|property| property.group == group && property.changed)
        });
        let properties: Vec<_> = all_properties
            .into_iter()
            .filter(|property| {
                property.matches(
                    &query,
                    self.settings_ui.appearance_modified_only,
                    self.settings_ui.appearance_group,
                    self.settings_ui.group,
                )
            })
            .collect();
        let empty = properties.is_empty();
        let count = properties.len();
        let mut list = settings_scroll_area("appearance-properties-scroll").gap_3();
        if !wide_preview && self.settings_ui.appearance_preview_open {
            list = list.child(self.appearance_preview());
        }
        if empty {
            let clear = view.clone();
            list = list.child(
                v_flex()
                    .py_6()
                    .gap_3()
                    .items_start()
                    .child(conduit::i18n::text_static("settings.appearance.no_matches"))
                    .child(
                        Button::new("appearance-clear-filters")
                            .small()
                            .outline()
                            .label(conduit::i18n::text_static(
                                "settings.appearance.clear_filters",
                            ))
                            .on_click(move |_, window, cx| {
                                clear.update(cx, |app, cx| app.clear_appearance_filters(window, cx))
                            }),
                    ),
            );
        } else {
            list = list.child(PropertyList {
                properties,
                changed_groups,
                view: view.clone(),
                stacked: property_width < 360. * scale,
            });
        }
        let body = h_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .items_start()
            .child(div().flex_1().min_w_0().h_full().child(list))
            .when(wide_preview, |body| {
                body.child(
                    div()
                        .id("appearance-preview-rail")
                        .debug_selector(|| "appearance-preview-rail".into())
                        .w(px(preview_width))
                        .h_full()
                        .flex_shrink_0()
                        .overflow_y_scroll()
                        .p_3()
                        .border_l_1()
                        .border_color(rgb(palette().border))
                        .bg(chrome_background(palette().surface))
                        .child(self.appearance_preview()),
                )
            });
        SettingsContent::new(body).with_toolbar(self.appearance_toolbar(
            count,
            !query.is_empty(),
            view,
        ))
    }

    pub(in crate::app) fn clear_appearance_filters(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_ui.appearance_modified_only = false;
        self.settings_ui.appearance_group = None;
        self.settings_ui.group = None;
        self.settings_ui
            .appearance_filter
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn appearance_toolbar(&self, count: usize, filtered: bool, view: Entity<Self>) -> AnyElement {
        let modified = view.clone();
        let groups = view.clone();
        let preview = view.clone();
        let clear = view;
        h_flex()
            .debug_selector(|| "appearance-filter-bar".into())
            .w_full()
            .min_w_0()
            .flex_wrap()
            .px_4()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                settings_dropdown("appearance-group-filter")
                    .label(self.settings_ui.appearance_group.map_or(
                        conduit::i18n::text_static("settings.appearance.all_groups"),
                        AppearanceGroup::label,
                    ))
                    .dropdown_menu(move |mut menu, _, _| {
                        for group in
                            std::iter::once(None).chain(AppearanceGroup::ALL.into_iter().map(Some))
                        {
                            let view = groups.clone();
                            menu = menu.item(
                                PopupMenuItem::new(group.map_or(
                                    conduit::i18n::text_static("settings.appearance.all_groups"),
                                    AppearanceGroup::label,
                                ))
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |app, cx| {
                                        app.settings_ui.appearance_group = group;
                                        app.settings_ui.group = None;
                                        cx.notify();
                                    })
                                }),
                            );
                        }
                        menu
                    }),
            )
            .child(
                Button::new("appearance-modified-only")
                    .debug_selector(|| "appearance-modified-only".into())
                    .small()
                    .ghost()
                    .selected(self.settings_ui.appearance_modified_only)
                    .label(conduit::i18n::text_static(
                        "settings.appearance.modified_only",
                    ))
                    .tooltip(conduit::i18n::text_static(
                        "settings.appearance.modified_hint",
                    ))
                    .on_click(move |_, _, cx| {
                        modified.update(cx, |app, cx| {
                            app.settings_ui.appearance_modified_only =
                                !app.settings_ui.appearance_modified_only;
                            cx.notify();
                        })
                    }),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::format(
                        "settings.appearance.result_count",
                        &[("count", count.to_string())],
                    )),
            )
            .when(self.settings_ui.group.is_some() || filtered, |bar| {
                bar.child(
                    Button::new("settings-show-full-page")
                        .debug_selector(|| "settings-show-full-page".into())
                        .small()
                        .ghost()
                        .label(conduit::i18n::text_static(
                            "settings.connections.view_full_page",
                        ))
                        .on_click(move |_, window, cx| {
                            clear.update(cx, |app, cx| app.clear_appearance_filters(window, cx))
                        }),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("appearance-toggle-preview")
                    .debug_selector(|| "appearance-toggle-preview".into())
                    .small()
                    .ghost()
                    .selected(self.settings_ui.appearance_preview_open)
                    .label(conduit::i18n::text_static(
                        "settings.appearance.typography_preview",
                    ))
                    .on_click(move |_, _, cx| {
                        preview.update(cx, |app, cx| {
                            app.settings_ui.appearance_preview_open =
                                !app.settings_ui.appearance_preview_open;
                            cx.notify();
                        })
                    }),
            )
            .into_any_element()
    }

    fn appearance_preview(&self) -> AnyElement {
        let ui = &self.store.ui;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(div().text_size(ui_text_size(14.)).font_semibold().child(
                conduit::i18n::text_static("settings.appearance.live_preview"),
            ))
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static(
                                "settings.appearance.application_interface",
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(FullIconName::Folder).small())
                            .child("Conduit"),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{} · {:.0} px",
                                ui.ui_font_family
                                    .as_deref()
                                    .unwrap_or(conduit::i18n::text_static(
                                        "settings.appearance.default_interface_preview"
                                    )),
                                ui.ui_font_size
                            )),
                    ),
            )
            .child(typography_previews(&self.chat_typography()))
            .children(
                [
                    AppearanceGroup::Interface,
                    AppearanceGroup::Chat,
                    AppearanceGroup::Code,
                ]
                .map(|group| {
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .font_medium()
                                .child(group.label()),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(group.scope()),
                        )
                }),
            )
            .into_any_element()
    }
}
