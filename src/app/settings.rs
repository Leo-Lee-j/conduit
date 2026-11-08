//! Settings workspace, preferences, and CLI configuration.

use super::*;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, Settings};

/// Keep groups available to the host so search can open a focused section
/// without depending on the library's private scrolling state.
pub(in crate::app) struct SettingPage {
    page: gpui_kit::component::setting::SettingPage,
    groups: Vec<SettingGroup>,
}

impl SettingPage {
    fn group(mut self, group: SettingGroup) -> Self {
        self.groups.push(group);
        self
    }

    fn groups(mut self, groups: impl IntoIterator<Item = SettingGroup>) -> Self {
        self.groups.extend(groups);
        self
    }

    fn into_page(
        self,
        group: Option<usize>,
        view: Entity<ConduitApp>,
    ) -> gpui_kit::component::setting::SettingPage {
        if let Some(group) = group.and_then(|index| self.groups.get(index)).cloned() {
            self.page
                .group(group)
                .title("")
                .header_style(&settings_actions_style())
                .title_suffix(move |_, _| {
                    let view = view.clone();
                    Button::new("settings-show-full-page")
                        .debug_selector(|| "settings-show-full-page".into())
                        .small()
                        .ghost()
                        .label(conduit::i18n::text_static(
                            "settings.connections.view_full_page",
                        ))
                        .on_click(move |_, window, cx| {
                            view.update(cx, |app, cx| {
                                app.select_settings_page(app.settings_ui.page, window, cx);
                            })
                        })
                })
        } else {
            self.page.groups(self.groups)
        }
    }
}

/// Navigation identifies each page; its body only shows settings and actions.
fn settings_page(title: &'static str, icon: FullIconName) -> SettingPage {
    let page = gpui_kit::component::setting::SettingPage::new(title)
        .icon(icon)
        .resettable(false)
        .header_style(&StyleRefinement::default().hidden());
    SettingPage {
        page,
        groups: vec![],
    }
}

fn settings_actions_style() -> StyleRefinement {
    StyleRefinement::default()
        .px(px(24.))
        .py(px(8.))
        .border_0()
        .flex_shrink_0()
        .bg(rgb(palette().canvas))
}

/// Section headings live in the card. Leaving the library's group title unset
/// keeps the sidebar flat instead of duplicating every section as a submenu.
fn settings_section(title: impl Into<SharedString>) -> SettingGroup {
    let title = title.into();
    let keywords = [title.clone()];
    SettingGroup::new().py(px(18.)).gap(px(20.)).item(
        SettingItem::render(move |_, _, _| {
            div()
                .w_full()
                .pb(px(8.))
                .border_b_1()
                .border_color(rgb(palette().border))
                .text_size(ui_text_size(14.))
                .font_semibold()
                .text_color(rgb(palette().text))
                .child(title.clone())
        })
        .keywords(keywords),
    )
}

/// A read-only setting value, right-aligned in the row.
pub(super) fn value_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        let tooltip_value = value.clone();
        div()
            .max_w(px(320.))
            .truncate()
            .text_size(ui_text_size(12.))
            .text_color(rgb(palette().muted))
            .id("settings-value")
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tooltip_value.clone()).build(window, cx)
            })
            .child(value.clone())
    })
}

pub(super) fn value_item(title: impl Into<SharedString>, value: String) -> SettingItem {
    SettingItem::new(title, value_field(value))
}

/// A wrapping paragraph, for status and hint text that cannot be truncated.
pub(super) fn paragraph_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        div()
            .w_full()
            .text_size(ui_text_size(12.))
            .line_height(relative(1.5))
            .text_color(rgb(palette().muted))
            .child(value.clone())
    })
}

/// A wrapping paragraph for errors, in the danger color.
pub(super) fn error_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        div()
            .w_full()
            .text_size(ui_text_size(12.))
            .line_height(relative(1.5))
            .text_color(rgb(palette().danger_text))
            .child(value.clone())
    })
}

/// A switch that saves to the store as soon as it changes.
pub(super) fn switch_field(
    view: Entity<ConduitApp>,
    get: fn(&ConduitApp) -> bool,
    set: fn(&mut ConduitApp, bool),
) -> SettingField<bool> {
    let read_view = view.clone();
    SettingField::switch(
        move |cx: &App| get(read_view.read(cx)),
        move |value, cx| {
            view.update(cx, |this, cx| {
                if !this.writable || this.closing {
                    return;
                }
                set(this, value);
                this.persist();
                cx.notify();
            });
        },
    )
}

/// A −/+ stepper for a bounded integer setting.
#[allow(clippy::too_many_arguments)]
pub(super) fn count_stepper_field(
    id: &'static str,
    value: usize,
    min: usize,
    max: usize,
    decrease_label: &'static str,
    increase_label: &'static str,
    disabled: bool,
    view: Entity<ConduitApp>,
    set: fn(&mut ConduitApp, usize),
) -> SettingField<SharedString> {
    let can_decrease = !disabled && value > min;
    let can_increase = !disabled && value < max;
    let decrease_value = value.saturating_sub(1).max(min);
    let increase_value = (value + 1).min(max);
    let decrease_view = view.clone();
    let increase_view = view;
    SettingField::render(move |_, _, _| {
        let decrease = decrease_view.clone();
        let increase = increase_view.clone();
        h_flex()
            .gap(px(2.))
            .p(px(2.))
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .items_center()
            .child(
                Button::new(format!("{id}-minus"))
                    .label("−")
                    .accessibility_label(decrease_label)
                    .small()
                    .ghost()
                    .disabled(!can_decrease)
                    .when(can_decrease, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        decrease.update(cx, |this, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            set(this, decrease_value);
                            this.persist();
                            cx.notify();
                        });
                    }),
            )
            .child(
                h_flex()
                    .w(px(36.))
                    .justify_center()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(if disabled {
                        palette().faint
                    } else {
                        palette().text
                    }))
                    .child(value.to_string()),
            )
            .child(
                Button::new(format!("{id}-plus"))
                    .label("+")
                    .accessibility_label(increase_label)
                    .small()
                    .ghost()
                    .disabled(!can_increase)
                    .when(can_increase, |button| button.cursor_pointer())
                    .on_click(move |_, _, cx| {
                        increase.update(cx, |this, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            set(this, increase_value);
                            this.persist();
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    })
}

/// One row of a font dropdown. Row zero always restores the default family.
#[derive(Clone)]
pub(super) struct FontChoice {
    pub(super) label: SharedString,
    pub(super) value: Option<SharedString>,
}

impl FontChoice {
    pub(super) fn default_entry(label: &'static str) -> Self {
        Self {
            label: label.into(),
            value: None,
        }
    }
}

impl SearchableListItem for FontChoice {
    type Value = Option<SharedString>;

    fn title(&self) -> SharedString {
        if self.value.is_none() {
            conduit::i18n::retained(&self.label).to_string().into()
        } else {
            self.label.clone()
        }
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Copy)]
pub(super) enum TypographyField {
    UiSize,
    ChatSize,
    ChatLineHeight,
    CodeSize,
}

impl TypographyField {
    pub(super) fn range(self) -> (f32, f32) {
        match self {
            Self::UiSize => (MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE),
            Self::ChatSize => (MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE),
            Self::ChatLineHeight => (MIN_CHAT_LINE_HEIGHT, MAX_CHAT_LINE_HEIGHT),
            Self::CodeSize => (MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE),
        }
    }
    pub(super) fn stepper_labels(self) -> (&'static str, &'static str) {
        match self {
            Self::UiSize => (
                conduit::i18n::text_static("settings.decrease_interface_font_size"),
                conduit::i18n::text_static("settings.increase_interface_font_size"),
            ),
            Self::ChatSize => (
                conduit::i18n::text_static("settings.decrease_chat_font_size"),
                conduit::i18n::text_static("settings.increase_chat_font_size"),
            ),
            Self::ChatLineHeight => (
                conduit::i18n::text_static("settings.decrease_chat_line_height"),
                conduit::i18n::text_static("settings.increase_chat_line_height"),
            ),
            Self::CodeSize => (
                conduit::i18n::text_static("settings.decrease_code_font_size"),
                conduit::i18n::text_static("settings.increase_code_font_size"),
            ),
        }
    }
    pub(super) fn display_value(self, value: f32) -> String {
        match self {
            Self::ChatLineHeight => format!("{value:.1}"),
            _ => format!("{value:.0}"),
        }
    }
}

pub(super) fn typography_field(
    id: &'static str,
    value: f32,
    disabled: bool,
    field: TypographyField,
    view: Entity<ConduitApp>,
) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        typography_stepper(id, value, disabled, field, view.clone())
    })
}

fn typography_stepper(
    id: &'static str,
    value: f32,
    disabled: bool,
    field: TypographyField,
    view: Entity<ConduitApp>,
) -> AnyElement {
    let (min, max) = field.range();
    let can_decrease = value > min;
    let can_increase = value < max;
    let (decrease_label, increase_label) = field.stepper_labels();
    let display = field.display_value(value);
    let decrease = view.clone();
    let increase = view;
    let enabled = !disabled;
    h_flex()
        .debug_selector(move || id.into())
        .gap(px(2.))
        .p(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface_strong))
        .items_center()
        .child(
            Button::new(format!("{id}-minus"))
                .debug_selector(move || format!("{id}-minus"))
                .label("−")
                .accessibility_label(decrease_label)
                .small()
                .ghost()
                .disabled(!enabled || !can_decrease)
                .when(enabled && can_decrease, |button| button.cursor_pointer())
                .on_click(move |_, _, cx| {
                    decrease.update(cx, |this, cx| {
                        this.adjust_typography(field, -1.0, cx);
                    });
                }),
        )
        .child(
            h_flex()
                .w(px(36.))
                .justify_center()
                .text_size(ui_text_size(12.))
                .text_color(rgb(if disabled {
                    palette().faint
                } else {
                    palette().text
                }))
                .child(display),
        )
        .child(
            Button::new(format!("{id}-plus"))
                .debug_selector(move || format!("{id}-plus"))
                .label("+")
                .accessibility_label(increase_label)
                .small()
                .ghost()
                .disabled(!enabled || !can_increase)
                .when(enabled && can_increase, |button| button.cursor_pointer())
                .on_click(move |_, _, cx| {
                    increase.update(cx, |this, cx| {
                        this.adjust_typography(field, 1.0, cx);
                    });
                }),
        )
        .into_any_element()
}

mod appearance;
#[cfg(all(test, feature = "ui-tests"))]
mod appearance_tests;
#[path = "settings/cli.rs"]
mod cli_settings;
mod config_editor;
#[path = "settings/connections.rs"]
mod connections;
mod defaults;
mod extension_management;
mod general;
mod management;
mod model_management;
use management::{engine_tabs, management_actions, management_badge};
#[path = "settings/models.rs"]
mod model_sync;
pub(super) use connections::ConnectionsUi;
#[path = "settings/extensions.rs"]
mod extensions_view;
#[path = "settings/navigation.rs"]
mod navigation;
mod search;
#[path = "settings/shortcuts.rs"]
mod shortcut_settings;
mod workspace;
#[cfg(all(test, feature = "ui-tests"))]
mod workspace_tests;
pub(super) use extensions_view::ExtensionsUi;
pub(super) use navigation::SettingsUi;

impl ConduitApp {
    pub(super) fn open_engine_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_ui.page = 3;
        self.settings_ui.group = None;
        self.settings_ui
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.open_settings(window, cx);
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_ui.open {
            self.settings_ui
                .search
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
            return;
        }
        if !self.open_tabs.contains(&files::DocumentTab::Settings) {
            self.open_tabs.push(files::DocumentTab::Settings);
        }
        self.settings_ui.focus_before_open = window.focused(cx).map(|focus| focus.downgrade());
        self.settings_ui.open = true;
        self.double_shift.cancel();
        self.settings_ui
            .search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        self.reload_all_models_async(false, window, cx);
        if matches!(self.settings_ui.page, 4 | 9) {
            self.ensure_extensions(window, cx, false);
        }
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let return_focus = self.settings_ui.focus_before_open.take();
        self.settings_ui.open = false;
        self.open_tabs
            .retain(|tab| *tab != files::DocumentTab::Settings);
        self.extensions_ui.probe = None;
        self.extensions_ui.cancel_mcp_detection();
        self.settings_ui
            .search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(active) = self.active_document() {
            self.select_document(active, window, cx);
        } else {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        // Selecting the document focuses its primary input; returning from settings
        // must restore the actual previous control, including a retained plugin.
        if let Some(focus) = return_focus.and_then(|focus| focus.upgrade()) {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn open_about_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let width = (f32::from(window.viewport_size().width) - 48.).clamp(0., 560.);
            let height = (f32::from(window.viewport_size().height) - 180.).max(100.);
            dialog
                .w(px(width))
                .centered()
                .title(conduit::i18n::text_static("settings.about"))
                .entrance(gpui_kit::component::dialog::DialogEntrance::Fade)
                .content(move |content, _, cx| {
                    content.child(
                        v_flex()
                            .id("about-update-body")
                            .max_h(px(height))
                            .overflow_y_scroll()
                            .gap_4()
                            .child(
                                h_flex().gap_3().items_center().child(app_icon(40.)).child(
                                    v_flex()
                                        .gap_1()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_size(ui_text_size(18.))
                                                .font_semibold()
                                                .text_color(rgb(palette().text))
                                                .child("Conduit"),
                                        )
                                        .child(
                                            div()
                                                .text_size(ui_text_size(11.))
                                                .text_color(rgb(palette().faint))
                                                .child(conduit::i18n::format(
                                                    "settings.version_windows",
                                                    &[(
                                                        "arg0",
                                                        env!("CARGO_PKG_VERSION").to_string(),
                                                    )],
                                                )),
                                        ),
                                ),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .line_height(relative(1.6))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::text_static(
                                        "settings.conduit_is_a_local_cli_workspace_that",
                                    )),
                            )
                            .child(view.read(cx).update_content(view.clone())),
                    )
                })
        });
        cx.notify();
    }
}
