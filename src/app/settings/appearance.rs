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

fn auto_collapse_field(view: Entity<ConduitApp>) -> SettingField<SharedString> {
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
                        div().w(px(104.)).flex_shrink_0().child(
                            NumberInput::new(&input)
                                .small()
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

fn theme_options(
    selected: AppearanceMode,
    view: Entity<ConduitApp>,
    disabled: bool,
    cx: &App,
) -> AnyElement {
    let ui = &view.read(cx).store.ui;
    let theme_option = |mode: AppearanceMode,
                        label: &'static str,
                        detail: &'static str,
                        icon: gpui_kit::assets::IconName,
                        id: &'static str|
     -> AnyElement {
        let view = view.clone();
        let active = selected == mode;
        let preview_mode = match mode {
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(cx.window_appearance()),
        };
        let name = if preview_mode.is_dark() {
            &ui.dark_theme
        } else {
            &ui.light_theme
        };
        let preview = theme_preview(name, preview_mode, cx);
        let label_color = if disabled {
            palette().faint
        } else {
            palette().text
        };
        let icon_color = if disabled {
            palette().faint
        } else if active {
            palette().text
        } else {
            palette().muted
        };
        let check_color = if disabled {
            palette().faint
        } else {
            palette().text
        };
        div()
            .w_full()
            .debug_selector(move || id.into())
            .child(
                Button::new(id)
                    .outline()
                    .h_auto()
                    .min_h(rems(8.))
                    .w_full()
                    .p_3()
                    .rounded(px(8.))
                    .border_color(rgb(if active {
                        palette().focus
                    } else {
                        palette().border
                    }))
                    .bg(rgb(if active {
                        palette().surface_strong
                    } else {
                        palette().surface
                    }))
                    .selected(active)
                    .accessibility_label(label)
                    .disabled(disabled)
                    .when(!disabled, |button| button.cursor_pointer())
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            if this.store.ui.appearance != mode {
                                this.store.ui.appearance = mode;
                                this.persist();
                            }
                            this.apply_appearance(window, cx);
                        });
                    })
                    .child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .whitespace_normal()
                            .gap_2()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(48.))
                                    .rounded(px(4.))
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(rgb(preview.border))
                                    .bg(rgb(preview.canvas))
                                    .child(
                                        div()
                                            .w(px(24.))
                                            .h_full()
                                            .bg(rgb(preview.sidebar_bg))
                                            .border_r_1()
                                            .border_color(rgb(preview.border)),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .px_2()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .w(relative(0.65))
                                                    .h(px(4.))
                                                    .rounded(px(2.))
                                                    .bg(rgb(preview.muted)),
                                            )
                                            .child(
                                                div()
                                                    .w_full()
                                                    .h(px(4.))
                                                    .rounded(px(2.))
                                                    .bg(rgb(preview.border)),
                                            )
                                            .child(
                                                div()
                                                    .w(relative(0.8))
                                                    .h(px(4.))
                                                    .rounded(px(2.))
                                                    .bg(rgb(preview.border)),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .child(Icon::new(icon).small().text_color(rgb(icon_color)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_size(ui_text_size(13.))
                                            .font_medium()
                                            .text_color(rgb(label_color))
                                            .child(label),
                                    )
                                    .when(active, |row| {
                                        row.child(
                                            Icon::new(gpui_kit::assets::IconName::Check)
                                                .small()
                                                .text_color(rgb(check_color)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(ui_text_size(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(detail),
                            ),
                    ),
            )
            .into_any_element()
    };
    h_flex()
        .w_full()
        .min_w_0()
        .flex_wrap()
        .items_start()
        .gap_3()
        .child(
            div()
                .flex_1()
                .flex_basis(rems(10.))
                .min_w_0()
                .child(theme_option(
                    AppearanceMode::System,
                    conduit::i18n::text_static("settings.appearance.system"),
                    conduit::i18n::text_static(
                        "settings.appearance.match_system_appearance_automatically",
                    ),
                    gpui_kit::assets::IconName::Monitor,
                    "appearance-theme-system",
                )),
        )
        .child(
            div()
                .flex_1()
                .flex_basis(rems(10.))
                .min_w_0()
                .child(theme_option(
                    AppearanceMode::Light,
                    conduit::i18n::text_static("settings.appearance.light"),
                    conduit::i18n::text_static("settings.appearance.bright_and_clear"),
                    gpui_kit::assets::IconName::Sun,
                    "appearance-theme-light",
                )),
        )
        .child(
            div()
                .flex_1()
                .flex_basis(rems(10.))
                .min_w_0()
                .child(theme_option(
                    AppearanceMode::Dark,
                    conduit::i18n::text_static("settings.appearance.dark"),
                    conduit::i18n::text_static("settings.appearance.soft_dark_background"),
                    gpui_kit::assets::IconName::Moon,
                    "appearance-theme-dark",
                )),
        )
        .into_any_element()
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
            .w(rems(14.))
            .max_w(relative(1.))
            .min_w_0()
            .child(
                Select::new(select)
                    .with_size(Size::Small)
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
                v_flex()
                    .flex_1()
                    .flex_basis(rems(16.))
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .child(conduit::i18n::text_static(title)),
                    )
                    .child(settings_description(conduit::i18n::text_static(description)).mt_0()),
            )
            .child(
                div()
                    .debug_selector(move || selector.into())
                    .w(rems(12.))
                    .max_w(relative(1.))
                    .min_w_0()
                    .child(
                        Select::new(select)
                            .with_size(Size::Small)
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

// These are subheadings inside the typography group, not new settings groups.
// Keep font selectors and steppers on the shared SettingItem layout.
fn font_settings_heading(
    target: FontTarget,
    disabled: bool,
    view: Entity<ConduitApp>,
) -> SettingItem {
    let (id, title, scope) = match target {
        FontTarget::Interface => (
            "appearance-interface-heading",
            conduit::i18n::text_static("settings.appearance.interface_typography"),
            conduit::i18n::text_static("settings.appearance.interface_typography_scope"),
        ),
        FontTarget::Chat => (
            "appearance-chat-heading",
            conduit::i18n::text_static("settings.appearance.chat_text"),
            conduit::i18n::text_static("settings.appearance.chat_typography_scope"),
        ),
        FontTarget::Code => (
            "appearance-code-heading",
            conduit::i18n::text_static("settings.appearance.code_blocks"),
            conduit::i18n::text_static("settings.appearance.code_typography_scope"),
        ),
    };
    SettingItem::render(move |_, _, _| {
        h_flex()
            .debug_selector(move || id.into())
            .w_full()
            .min_w_0()
            .pt_3()
            .gap_3()
            .flex_wrap()
            .items_start()
            .child(
                v_flex()
                    .flex_1()
                    .flex_basis(rems(16.))
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(title),
                    )
                    .child(settings_description(scope).mt_0()),
            )
            .when(matches!(target, FontTarget::Interface), |row| {
                let view = view.clone();
                row.child(
                    Button::new("appearance-ui-reset")
                        .debug_selector(|| "appearance-ui-reset".into())
                        .label(conduit::i18n::text_static(
                            "settings.appearance.restore_interface_defaults",
                        ))
                        .tooltip(conduit::i18n::text_static(
                            "settings.appearance.reset_only_the_interface_font_and_size",
                        ))
                        .small()
                        .ghost()
                        .disabled(disabled)
                        .when(!disabled, |button| button.cursor_pointer())
                        .on_click(move |_, window, cx| {
                            view.update(cx, |app, cx| app.reset_ui_typography(window, cx));
                        }),
                )
            })
    })
    .keywords([title, scope])
    .disabled(disabled)
}

// A local, inert source sample: rendering this never executes the JavaScript.
const CODE_TYPOGRAPHY_SAMPLE: &str = r#"```javascript
function previewTypography(settings) {
  return {
    fontFamily: settings.codeFont,
    fontSize: settings.codeSize,
  };
}
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
            .rounded(px(8.))
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
        .pt_4()
        .border_t_1()
        .border_color(rgb(palette().border))
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_semibold()
                        .text_color(rgb(palette().text))
                        .child(conduit::i18n::text_static(
                            "settings.appearance.typography_preview",
                        )),
                )
                .child(
                    settings_description(conduit::i18n::text_static(
                        "settings.appearance.static_preview_description",
                    ))
                    .mt_0(),
                ),
        )
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
                        conduit::i18n::text_static("settings.appearance.chat_preview_sample"),
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

    pub(in crate::app) fn reset_typography(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        self.store.ui.chat_font_family = None;
        self.store.ui.chat_font_size = 15.0;
        self.store.ui.chat_line_height = 1.5;
        self.store.ui.code_font_family = None;
        self.store.ui.code_font_size = 13.0;
        self.store.ui.code_copy_button = true;
        self.store.ui.code_syntax_highlight = true;
        self.store.ui.code_line_numbers = false;
        for select in [&self.chat_font_select, &self.code_font_select] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::new(0)), window, cx);
            });
        }
        self.apply_code_font(cx);
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn settings_appearance_page(
        &self,
        disabled: bool,
        view: Entity<Self>,
    ) -> SettingPage {
        let selected = self.store.ui.appearance;
        let theme_view = view.clone();
        let chat = self.chat_typography();
        let ui_font = font_select_item(view.clone(), FontTarget::Interface, disabled);
        let ui_size = typography_field(
            "ui-font-size",
            self.store
                .ui
                .ui_font_size
                .clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE),
            disabled,
            TypographyField::UiSize,
            view.clone(),
        );
        let icon_view = view.clone();
        let icon_size = SettingField::render(move |_, _, cx| {
            icon_size_options(
                icon_view.read(cx).store.ui.icon_size,
                icon_view.clone(),
                disabled,
            )
        });
        let chat_font = font_select_item(view.clone(), FontTarget::Chat, disabled);
        let chat_size = typography_field(
            "chat-font-size",
            chat.body_size,
            disabled,
            TypographyField::ChatSize,
            view.clone(),
        );
        let chat_line_height = typography_field(
            "chat-line-height",
            chat.line_height,
            disabled,
            TypographyField::ChatLineHeight,
            view.clone(),
        );
        let code_font = font_select_item(view.clone(), FontTarget::Code, disabled);
        let code_size = typography_field(
            "code-font-size",
            chat.code_size,
            disabled,
            TypographyField::CodeSize,
            view.clone(),
        );
        let preview_view = view.clone();
        let reset_view = view.clone();
        let reset = SettingItem::render(move |_, _, _| {
            let view = reset_view.clone();
            h_flex()
                .w_full()
                .min_w_0()
                .gap_3()
                .flex_wrap()
                .items_start()
                .child(
                    div().flex_1().flex_basis(rems(16.)).min_w_0().child(
                        settings_description(conduit::i18n::text_static(
                            "settings.appearance.reset_chat_code_scope",
                        ))
                        .mt_0(),
                    ),
                )
                .child(
                    Button::new("appearance-typography-reset")
                        .debug_selector(|| "appearance-typography-reset".into())
                        .label(conduit::i18n::text_static(
                            "settings.appearance.restore_chat_code_defaults",
                        ))
                        .small()
                        .ghost()
                        .disabled(disabled)
                        .when(!disabled, |button| button.cursor_pointer())
                        .on_click(move |_, window, cx| {
                            view.update(cx, |app, cx| app.reset_typography(window, cx));
                        }),
                )
        })
        .keywords([
            conduit::i18n::text_static("settings.appearance.restore_chat_code_defaults"),
            conduit::i18n::text_static("settings.appearance.reset_chat_code_scope"),
        ])
        .disabled(disabled);
        let code_copy = switch_field(
            view.clone(),
            |app| app.store.ui.code_copy_button,
            |app, value| app.store.ui.code_copy_button = value,
        );
        let code_line_numbers = switch_field(
            view.clone(),
            |app| app.store.ui.code_line_numbers,
            |app, value| app.store.ui.code_line_numbers = value,
        );
        let code_syntax = switch_field(
            view.clone(),
            |app| app.store.ui.code_syntax_highlight,
            |app, value| app.store.ui.code_syntax_highlight = value,
        );
        let auto_sort = switch_field(
            view.clone(),
            |app| app.store.ui.auto_sort_workspaces,
            |app, value| app.store.ui.auto_sort_workspaces = value,
        );
        let auto_collapse = auto_collapse_field(view.clone());

        // Search/navigation depend on these indices: 0 appearance, 1 typography,
        // 2 background. Font subsections and previews must not add top-level groups.
        settings_page(
            conduit::i18n::text_static("settings.appearance.appearance"),
            FullIconName::Palette,
        )
        .groups(vec![
            settings_section(conduit::i18n::text_static("settings.appearance.appearance"))
                .item(
                    SettingItem::render(move |_, _, cx| {
                        theme_options(selected, theme_view.clone(), disabled, cx)
                    })
                    .keywords([
                        conduit::i18n::text_static("settings.appearance.theme"),
                        conduit::i18n::text_static("settings.appearance.light"),
                        conduit::i18n::text_static("settings.appearance.dark"),
                        conduit::i18n::text_static("settings.appearance.system"),
                    ]),
                )
                .item(
                    SettingItem::new(
                        conduit::i18n::text_static("settings.appearance.light_theme"),
                        theme_select_field(view.clone(), gpui_kit::component::ThemeMode::Light, disabled),
                    )
                    .description(settings_description(conduit::i18n::text_static(
                        "settings.appearance.light_theme_description",
                    )))
                    .disabled(disabled),
                )
                .item(
                    SettingItem::new(
                        conduit::i18n::text_static("settings.appearance.dark_theme"),
                        theme_select_field(view.clone(), gpui_kit::component::ThemeMode::Dark, disabled),
                    )
                    .description(settings_description(conduit::i18n::text_static(
                        "settings.appearance.dark_theme_description",
                    )))
                    .disabled(disabled),
                )
                .item(
                    SettingItem::render(|_, _, _| {
                        div()
                            .pt_3()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(conduit::i18n::text_static(
                                "settings.appearance.application_interface",
                            ))
                    })
                    .keywords([conduit::i18n::text_static(
                        "settings.appearance.application_interface",
                    )]),
                )
                .item(
                    SettingItem::new(
                        conduit::i18n::text_static("settings.appearance.auto_sort_workspaces"),
                        auto_sort,
                    )
                    .description(settings_description(conduit::i18n::text_static(
                        "settings.appearance.auto_sort_workspaces_description",
                    )))
                    .disabled(disabled),
                )
                .item(
                    SettingItem::new(
                        conduit::i18n::text_static("settings.appearance.auto_collapse_inactive_workspaces"),
                        auto_collapse,
                    )
                    .description(settings_description(conduit::i18n::text_static(
                        "settings.appearance.auto_collapse_inactive_workspaces_description",
                    )))
                    .disabled(disabled),
                )
                .footer(|_, _| {
                    settings_description(conduit::i18n::text_static(
                        "settings.appearance.applies_immediately_system_mode_follows_windows_appearance",
                    ))
                    .mt_0()
                }),
            settings_section(conduit::i18n::text_static("settings.appearance.typography"))
                .items(vec![
                    SettingItem::render(|_, _, _| {
                        settings_description(conduit::i18n::text_static(
                            "settings.appearance.changes_apply_immediately_and_are_saved_automatically",
                        ))
                        .mt_0()
                    })
                    .keywords([
                        conduit::i18n::text_static("settings.appearance.typography"),
                        conduit::i18n::text_static("settings.appearance.typography_preview"),
                        conduit::i18n::text_static("settings.appearance.chat_preview"),
                        conduit::i18n::text_static("settings.appearance.code_preview"),
                        conduit::i18n::text_static("mermaid.preview"),
                    ]),
                    font_settings_heading(FontTarget::Interface, disabled, view.clone()),
                    ui_font,
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.interface_font_size"), ui_size)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.12_20_px_default_16_scales_interface")))
                        .keywords([conduit::i18n::text_static("settings.appearance.application"), conduit::i18n::text_static("settings.appearance.interface"), conduit::i18n::text_static("settings.appearance.font_size"), conduit::i18n::text_static("settings.appearance.size"), "font", "size"])
                        .disabled(disabled),
                    SettingItem::new(
                        conduit::i18n::text_static("settings.appearance.functional_icon_size"),
                        icon_size,
                    )
                    .description(settings_description(conduit::i18n::text_static(
                        "settings.appearance.functional_icon_size_description",
                    )))
                    .keywords([
                        conduit::i18n::text_static("settings.appearance.functional_icon_size"),
                        "icon",
                        "sidebar",
                        "toolbar",
                    ])
                    .disabled(disabled),
                    font_settings_heading(FontTarget::Chat, disabled, view.clone()),
                    chat_font,
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.font_size"), chat_size)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.12_20_px_default_15")))
                        .disabled(disabled),
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.line_height"), chat_line_height)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.1_2_2_0_default_1_5")))
                        .disabled(disabled),
                    font_settings_heading(FontTarget::Code, disabled, view.clone()),
                    code_font,
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.code_font_size"), code_size)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.10_18_px_default_13")))
                        .disabled(disabled),
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.copy_button"), code_copy)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.show_a_copy_button_in_the_upper")))
                        .disabled(disabled),
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.line_numbers"), code_line_numbers)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.show_line_numbers_beside_code_blocks_excluded")))
                        .disabled(disabled),
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.syntax_highlighting"), code_syntax)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.color_code_by_language_when_disabled_use")))
                        .disabled(disabled),
                    reset,
                ])
                // Previews sit outside the settings surface; they contain no
                // preference actions and remain readable in read-only mode.
                .footer(move |_, cx| {
                    typography_previews(&preview_view.read(cx).chat_typography())
                }),
            settings_section(conduit::i18n::text_static("wallpaper.title")).item(
                SettingItem::render(move |_, _, cx| view.read(cx).background_controls(view.clone()))
                    .keywords([conduit::i18n::text_static("wallpaper.title"), "Wallhaven", "wallpaper", "background"]),
            ),
        ])
    }
}
