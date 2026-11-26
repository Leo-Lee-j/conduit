//! Appearance settings.

use super::*;
use crate::store::UiIconSize;
use gpui_kit::component::{input::NumberInput, switch::Switch};

impl ConduitApp {
    pub(in crate::app) fn on_auto_collapse_days_change(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        if matches!(
            event,
            InputEvent::Change | InputEvent::Blur | InputEvent::PressEnter { .. }
        ) {
            if let Ok(days) = input.read(cx).value().parse::<u32>()
                && days > 0
                && self.store.ui.auto_collapse_inactive_days != days
            {
                self.store.ui.auto_collapse_inactive_days = days;
                self.persist();
                cx.notify();
            }
            if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                let value = self.store.ui.auto_collapse_inactive_days.max(1).to_string();
                if input.read(cx).value().as_ref() != value {
                    input.update(cx, |input, cx| input.set_value(value, window, cx));
                }
            }
        }
    }
}

pub(super) fn auto_collapse_field(view: Entity<ConduitApp>) -> SettingField<SharedString> {
    SettingField::render(move |options, _, cx| {
        let app = view.read(cx);
        let enabled = app.store.ui.auto_collapse_inactive_workspaces;
        let input = app.settings_ui.auto_collapse_inactive_days.clone();
        let view = view.clone();
        h_flex()
            .debug_selector(|| "auto-collapse-inactive-controls".into())
            .gap_3()
            .items_center()
            .child(
                h_flex()
                    .debug_selector(|| "auto-collapse-inactive-days".into())
                    .gap_2()
                    .child(
                        div().w(ui_text_size(104.)).flex_shrink_0().child(
                            NumberInput::new(&input)
                                .settings_control()
                                .accessibility_label(conduit::i18n::text_static(
                                    "settings.appearance.auto_collapse_inactive_days",
                                ))
                                .disabled(options.is_disabled() || !enabled),
                        ),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::text_static("settings.appearance.days")),
                    ),
            )
            .child(
                Switch::new("auto-collapse-inactive-switch")
                    .accessibility_label(conduit::i18n::text_static(
                        "settings.appearance.auto_collapse_inactive_workspaces",
                    ))
                    .checked(enabled)
                    .with_size(options.size())
                    .disabled(options.is_disabled())
                    .on_click(move |checked, _, cx| {
                        view.update(cx, |app, cx| {
                            if !app.writable || app.closing {
                                return;
                            }
                            app.store.ui.auto_collapse_inactive_workspaces = *checked;
                            app.persist();
                            cx.notify();
                        });
                    }),
            )
    })
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

#[cfg(all(test, feature = "ui-tests"))]
#[path = "appearance_redesign_tests.rs"]
mod redesign_tests;

pub(super) mod editor;

fn theme_options(selected: AppearanceMode, view: Entity<ConduitApp>, disabled: bool) -> AnyElement {
    let mut choices = h_flex().gap_1().flex_wrap();
    for (mode, key, id) in [
        (
            AppearanceMode::System,
            "settings.appearance.system",
            "appearance-theme-system",
        ),
        (
            AppearanceMode::Light,
            "settings.appearance.light",
            "appearance-theme-light",
        ),
        (
            AppearanceMode::Dark,
            "settings.appearance.dark",
            "appearance-theme-dark",
        ),
    ] {
        let view = view.clone();
        choices = choices.child(
            Button::new(id)
                .debug_selector(move || id.into())
                .small()
                .ghost()
                .selected(selected == mode)
                .label(conduit::i18n::text_static(key))
                .disabled(disabled)
                .on_click(move |_, window, cx| {
                    view.update(cx, |app, cx| {
                        if !app.writable || app.closing || app.store.ui.appearance == mode {
                            return;
                        }
                        app.store.ui.appearance = mode;
                        app.apply_appearance(window, cx);
                        app.persist();
                    })
                }),
        );
    }
    choices.into_any_element()
}

#[derive(Clone, Copy)]
enum FontTarget {
    Interface,
    Chat,
    Code,
}

fn icon_size_options(selected: UiIconSize, view: Entity<ConduitApp>, disabled: bool) -> AnyElement {
    let mut options = h_flex().gap_1();
    for (index, size) in UiIconSize::ALL.into_iter().enumerate() {
        let view = view.clone();
        options = options.child(
            Button::new(("appearance-functional-icon-size", index))
                .debug_selector(move || format!("appearance-functional-icon-size-{index}"))
                .small()
                .ghost()
                .selected(size == selected)
                .label(size.label())
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click(move |_, _, cx| {
                    view.update(cx, |app, cx| app.set_ui_icon_size(size, cx));
                }),
        );
    }
    options.into_any_element()
}

fn theme_select_field(
    view: Entity<ConduitApp>,
    mode: gpui_kit::component::ThemeMode,
    disabled: bool,
) -> SettingField<SharedString> {
    SettingField::render(move |_, _, cx| {
        let app = view.read(cx);
        let (select, selector, label) = if mode.is_dark() {
            (
                &app.dark_theme_select,
                "appearance-dark-theme",
                conduit::i18n::text_static("settings.appearance.dark_theme"),
            )
        } else {
            (
                &app.light_theme_select,
                "appearance-light-theme",
                conduit::i18n::text_static("settings.appearance.light_theme"),
            )
        };
        div()
            .debug_selector(move || selector.into())
            .w(rems(12.))
            .max_w(relative(1.))
            .min_w_0()
            .child(
                Select::new(select)
                    .settings_control()
                    .w_full()
                    .menu_width(px(280.))
                    .menu_max_h(px(320.))
                    .search_placeholder(conduit::i18n::text_static(
                        "settings.appearance.search_themes",
                    ))
                    .accessibility_label(label)
                    .disabled(disabled),
            )
    })
}

/// A font row wraps before its label and selector compete for the same space.
/// The library's fixed 480px stacking breakpoint does not account for UI rems.
fn font_select_item(view: Entity<ConduitApp>, target: FontTarget, disabled: bool) -> SettingItem {
    let (title, description) = match target {
        FontTarget::Interface => (
            "settings.appearance.interface_font",
            "settings.appearance.used_for_the_sidebar_tabs_menus_buttons",
        ),
        FontTarget::Chat => (
            "settings.appearance.body_font",
            "settings.appearance.used_for_messages_and_input_defaults_to",
        ),
        FontTarget::Code => (
            "settings.appearance.code_font",
            "settings.appearance.used_for_code_blocks_and_tool_output",
        ),
    };
    SettingItem::render(move |_, _, cx| {
        let app = view.read(cx);
        let (select, selector, label) = match target {
            FontTarget::Interface => (
                &app.ui_font_select,
                "appearance-ui-font",
                conduit::i18n::text_static("settings.appearance.application_interface_font"),
            ),
            FontTarget::Chat => (
                &app.chat_font_select,
                "appearance-chat-font",
                conduit::i18n::text_static("settings.appearance.chat_body_font"),
            ),
            FontTarget::Code => (
                &app.code_font_select,
                "appearance-code-font",
                conduit::i18n::text_static("settings.appearance.code_font"),
            ),
        };
        h_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .flex_wrap()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .flex_basis(rems(8.))
                    .min_w_0()
                    .text_size(ui_text_size(14.))
                    .child(conduit::i18n::text_static(title)),
            )
            .child(
                div()
                    .debug_selector(move || selector.into())
                    .w(rems(12.))
                    .max_w(relative(1.))
                    .min_w_0()
                    .child(
                        Select::new(select)
                            .settings_control()
                            .w_full()
                            .menu_width(px(260.))
                            .menu_max_h(px(320.))
                            .search_placeholder(conduit::i18n::text_static(
                                "settings.appearance.search_fonts",
                            ))
                            .accessibility_label(label)
                            .disabled(disabled),
                    ),
            )
    })
    .keywords([
        conduit::i18n::text_static(title),
        conduit::i18n::text_static(description),
        conduit::i18n::text_static("settings.appearance.application"),
        conduit::i18n::text_static("settings.appearance.interface"),
        conduit::i18n::text_static("settings.appearance.font"),
        "UI",
        "font",
    ])
    .disabled(disabled)
}

// A local, inert source sample: rendering this never executes the JavaScript.
const CODE_TYPOGRAPHY_SAMPLE: &str = r#"```javascript
const sizes = [13, 15];
const label = sizes
  .map(n => `${n}px`)
  .join(' / ');
```"#;

// The preview accepts only resolved typography, not sessions, input state or
// services. Stable TextView IDs reuse the existing native Markdown renderer;
// selectable=false leaves the editor's selection alone.
fn typography_previews(typography: &ChatTypography) -> AnyElement {
    let preview_panel = |id: &'static str, title: &'static str, body: AnyElement| {
        v_flex()
            .debug_selector(move || id.into())
            .flex_1()
            .flex_basis(rems(18.))
            .min_w_0()
            .max_w(relative(1.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .text_size(ui_text_size(12.))
                    .font_medium()
                    .text_color(rgb(palette().muted))
                    .child(title),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .p_3()
                    .text_color(rgb(palette().text))
                    .child(body),
            )
            .into_any_element()
    };
    v_flex()
        .debug_selector(|| "appearance-typography-previews".into())
        .w_full()
        .min_w_0()
        .gap_3()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .items_start()
                .gap_3()
                .child(preview_panel(
                    "appearance-chat-preview-panel",
                    conduit::i18n::text_static("settings.appearance.chat_preview"),
                    chat_markdown_view(
                        "appearance-chat-preview".into(),
                        conduit::i18n::text_static("settings.appearance.compact_preview_sample"),
                        typography,
                        "appearance-preview-chat",
                        false,
                    )
                    .into_any_element(),
                ))
                .child(preview_panel(
                    "appearance-code-preview-panel",
                    conduit::i18n::text_static("settings.appearance.code_preview"),
                    chat_markdown_view(
                        "appearance-code-preview".into(),
                        CODE_TYPOGRAPHY_SAMPLE,
                        typography,
                        "appearance-preview-code",
                        false,
                    )
                    .into_any_element(),
                )),
        )
        .into_any_element()
}

impl ConduitApp {
    pub(in crate::app) fn select_theme(
        &mut self,
        mode: gpui_kit::component::ThemeMode,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable
            || self.closing
            || gpui_kit::component::ThemeRegistry::global(cx)
                .themes()
                .get(name)
                .is_none_or(|theme| theme.mode != mode)
        {
            return;
        }
        let selected = if mode.is_dark() {
            &mut self.store.ui.dark_theme
        } else {
            &mut self.store.ui.light_theme
        };
        if selected != name {
            *selected = name.into();
            self.persist();
        }
        self.apply_appearance(window, cx);
    }

    pub(in crate::app) fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.store.ui.appearance {
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(window.appearance()),
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
        };
        apply_theme(
            mode,
            &self.store.ui.light_theme,
            &self.store.ui.dark_theme,
            cx,
        );
        super::super::theme::apply_icon_size(self.store.ui.icon_size, cx);
        // Theme changes reload font tokens. Restore both user choices afterward.
        self.apply_ui_font(cx);
        self.apply_code_font(cx);
        cx.notify();
    }

    pub(in crate::app) fn set_ui_icon_size(&mut self, size: UiIconSize, cx: &mut Context<Self>) {
        if !self.writable || self.closing || self.store.ui.icon_size == size {
            return;
        }
        self.store.ui.icon_size = size;
        super::super::theme::apply_icon_size(size, cx);
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn apply_ui_font(&mut self, cx: &mut Context<Self>) {
        install_ui_font(
            cx,
            self.store.ui.ui_font_family.as_deref(),
            &self.default_ui_font,
            self.store.ui.ui_font_size,
        );
    }

    pub(in crate::app) fn apply_code_font(&mut self, cx: &mut Context<Self>) {
        install_code_font(
            cx,
            self.store.ui.code_font_family.as_deref(),
            &self.default_mono_font,
            self.store.ui.code_font_size,
        );
    }

    pub(in crate::app) fn chat_typography(&self) -> ChatTypography {
        ChatTypography {
            body_font: Some(
                self.store
                    .ui
                    .chat_font_family
                    .as_deref()
                    .map(SharedString::from)
                    .unwrap_or_else(|| self.default_ui_font.clone()),
            ),
            body_size: self
                .store
                .ui
                .chat_font_size
                .clamp(MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE),
            line_height: self
                .store
                .ui
                .chat_line_height
                .clamp(MIN_CHAT_LINE_HEIGHT, MAX_CHAT_LINE_HEIGHT),
            code_font: self
                .store
                .ui
                .code_font_family
                .as_deref()
                .map(SharedString::from)
                .unwrap_or_else(|| self.default_mono_font.clone()),
            code_size: self
                .store
                .ui
                .code_font_size
                .clamp(MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE),
            code_copy: self.store.ui.code_copy_button,
            syntax_highlight: self.store.ui.code_syntax_highlight,
            code_line_numbers: self.store.ui.code_line_numbers,
        }
    }

    pub(in crate::app) fn adjust_typography(
        &mut self,
        field: TypographyField,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        match field {
            TypographyField::UiSize => {
                let value = self
                    .store
                    .ui
                    .ui_font_size
                    .clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE);
                self.store.ui.ui_font_size =
                    (value + delta).clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE);
                self.apply_ui_font(cx);
            }
            TypographyField::ChatSize => {
                let value = self
                    .store
                    .ui
                    .chat_font_size
                    .clamp(MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE);
                self.store.ui.chat_font_size =
                    (value + delta).clamp(MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE);
            }
            TypographyField::ChatLineHeight => {
                let value = self
                    .store
                    .ui
                    .chat_line_height
                    .clamp(MIN_CHAT_LINE_HEIGHT, MAX_CHAT_LINE_HEIGHT);
                // Keep one decimal so repeated steps do not drift.
                self.store.ui.chat_line_height =
                    (value * 10.0 + delta).clamp(12.0, 20.0).round() / 10.0;
            }
            TypographyField::CodeSize => {
                let value = self
                    .store
                    .ui
                    .code_font_size
                    .clamp(MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE);
                self.store.ui.code_font_size =
                    (value + delta).clamp(MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE);
                self.apply_code_font(cx);
            }
        }
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn reset_ui_typography(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        self.store.ui.ui_font_family = None;
        self.store.ui.ui_font_size = DEFAULT_UI_FONT_SIZE;
        self.ui_font_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.apply_ui_font(cx);
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn reset_chat_typography(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let defaults = crate::store::UiSettings::default();
        self.store.ui.chat_font_family = defaults.chat_font_family;
        self.store.ui.chat_font_size = defaults.chat_font_size;
        self.store.ui.chat_line_height = defaults.chat_line_height;
        self.chat_font_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn reset_code_typography(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing {
            return;
        }
        let defaults = crate::store::UiSettings::default();
        self.store.ui.code_font_family = defaults.code_font_family;
        self.store.ui.code_font_size = defaults.code_font_size;
        self.store.ui.code_copy_button = defaults.code_copy_button;
        self.store.ui.code_syntax_highlight = defaults.code_syntax_highlight;
        self.store.ui.code_line_numbers = defaults.code_line_numbers;
        self.code_font_select.update(cx, |select, cx| {
            select.set_selected_index(Some(IndexPath::new(0)), window, cx);
        });
        self.apply_code_font(cx);
        self.persist();
        cx.notify();
    }
}
