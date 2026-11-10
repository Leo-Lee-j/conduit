//! Appearance settings.

use super::*;

fn theme_options(selected: AppearanceMode, view: Entity<ConduitApp>, disabled: bool) -> AnyElement {
    let theme_option = |mode: AppearanceMode,
                        label: &'static str,
                        detail: &'static str,
                        icon: gpui_kit::assets::IconName,
                        id: &'static str|
     -> AnyElement {
        let view = view.clone();
        let active = selected == mode;
        let preview = palette_for_mode(match mode {
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
            AppearanceMode::System if is_dark_mode() => gpui_kit::component::ThemeMode::Dark,
            AppearanceMode::System => gpui_kit::component::ThemeMode::Light,
        });
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
                    .h(px(128.))
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
        .gap_3()
        .child(div().flex_1().min_w_0().child(theme_option(
            AppearanceMode::System,
            conduit::i18n::text_static("settings.appearance.system"),
            conduit::i18n::text_static("settings.appearance.match_system_appearance_automatically"),
            gpui_kit::assets::IconName::Monitor,
            "appearance-theme-system",
        )))
        .child(div().flex_1().min_w_0().child(theme_option(
            AppearanceMode::Light,
            conduit::i18n::text_static("settings.appearance.light"),
            conduit::i18n::text_static("settings.appearance.bright_and_clear"),
            gpui_kit::assets::IconName::Sun,
            "appearance-theme-light",
        )))
        .child(div().flex_1().min_w_0().child(theme_option(
            AppearanceMode::Dark,
            conduit::i18n::text_static("settings.appearance.dark"),
            conduit::i18n::text_static("settings.appearance.soft_dark_background"),
            gpui_kit::assets::IconName::Moon,
            "appearance-theme-dark",
        )))
        .into_any_element()
}

#[derive(Clone, Copy)]
enum FontTarget {
    Interface,
    Chat,
    Code,
}

fn font_select_field(
    view: Entity<ConduitApp>,
    target: FontTarget,
    disabled: bool,
) -> SettingField<SharedString> {
    SettingField::render(move |_, _, cx| {
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
        div()
            .debug_selector(move || selector.into())
            .w(px(192.))
            .flex_shrink_0()
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
            )
            .into_any_element()
    })
}

impl ConduitApp {
    pub(in crate::app) fn apply_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.store.ui.appearance {
            AppearanceMode::System => gpui_kit::component::ThemeMode::from(window.appearance()),
            AppearanceMode::Light => gpui_kit::component::ThemeMode::Light,
            AppearanceMode::Dark => gpui_kit::component::ThemeMode::Dark,
        };
        set_palette(mode);
        if gpui_kit::component::Theme::global(cx).mode != mode {
            gpui_kit::component::Theme::change(mode, Some(window), cx);
        }
        // Theme changes reload font tokens. Restore both user choices afterward.
        self.apply_ui_font(cx);
        self.apply_code_font(cx);
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
        let ui_font = font_select_field(view.clone(), FontTarget::Interface, disabled);
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
        let ui_reset_view = view.clone();
        let ui_reset = SettingField::render(move |_, _, _| {
            let view = ui_reset_view.clone();
            Button::new("appearance-ui-reset")
                .debug_selector(|| "appearance-ui-reset".into())
                .label(conduit::i18n::text_static(
                    "settings.appearance.restore_defaults",
                ))
                .small()
                .ghost()
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click(move |_, window, cx| {
                    view.update(cx, |app, cx| app.reset_ui_typography(window, cx));
                })
        });
        let chat_font = font_select_field(view.clone(), FontTarget::Chat, disabled);
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
        let code = self.chat_typography();
        let code_font = font_select_field(view.clone(), FontTarget::Code, disabled);
        let code_size = typography_field(
            "code-font-size",
            code.code_size,
            disabled,
            TypographyField::CodeSize,
            view.clone(),
        );
        let preview_view = view.clone();
        let reset_view = view.clone();
        let reset = SettingField::render(move |_, _window, _| {
            let reset_view = reset_view.clone();
            Button::new("appearance-typography-reset")
                .label(conduit::i18n::text_static(
                    "settings.appearance.restore_defaults",
                ))
                .small()
                .ghost()
                .disabled(disabled)
                .when(!disabled, |button| button.cursor_pointer())
                .on_click(move |_, window, cx| {
                    reset_view.update(cx, |this, cx| {
                        this.reset_typography(window, cx);
                    });
                })
        });
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
        let auto_collapse = switch_field(
            view.clone(),
            |app| app.store.ui.auto_collapse_inactive_workspaces,
            |app, value| app.store.ui.auto_collapse_inactive_workspaces = value,
        );
        settings_page(conduit::i18n::text_static("settings.appearance.appearance"), FullIconName::Palette).groups(vec![
            settings_section(conduit::i18n::text_static("settings.appearance.theme"))
                .item(
                    SettingItem::render(move |_, _, _| {
                        theme_options(selected, theme_view.clone(), disabled)
                    })
                    .keywords([conduit::i18n::text_static("settings.appearance.theme"), conduit::i18n::text_static("settings.appearance.light"), conduit::i18n::text_static("settings.appearance.dark"), conduit::i18n::text_static("settings.appearance.system")]),
                )
                .footer(|_, _| settings_description(conduit::i18n::text_static("settings.appearance.applies_immediately_system_mode_follows_windows_appearance")).mt_0()),
            settings_section(conduit::i18n::text_static("settings.appearance.application_interface"))
                .item(
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.auto_collapse_inactive_workspaces"), auto_collapse)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.auto_collapse_inactive_workspaces_description")))
                        .disabled(disabled),
                )
                .item(
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.interface_font"), ui_font)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.used_for_the_sidebar_tabs_menus_buttons")))
                        .keywords([conduit::i18n::text_static("settings.appearance.application"), conduit::i18n::text_static("settings.appearance.interface"), "UI", conduit::i18n::text_static("settings.appearance.font"), "font"])
                        .disabled(disabled),
                )
                .item(
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.interface_font_size"), ui_size)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.12_20_px_default_16_scales_interface")))
                        .keywords([conduit::i18n::text_static("settings.appearance.application"), conduit::i18n::text_static("settings.appearance.interface"), conduit::i18n::text_static("settings.appearance.font_size"), conduit::i18n::text_static("settings.appearance.size"), "font", "size"])
                        .disabled(disabled),
                )
                .item(
                    SettingItem::new(conduit::i18n::text_static("settings.appearance.restore_defaults"), ui_reset)
                        .description(settings_description(conduit::i18n::text_static("settings.appearance.reset_only_the_interface_font_and_size")))
                        .disabled(disabled),
                )
                .footer(|_, _| settings_description(conduit::i18n::text_static("settings.appearance.changes_apply_immediately_and_are_saved_automatically")).mt_0()),
            settings_section(conduit::i18n::text_static("settings.appearance.chat_text")).items(vec![
                SettingItem::new(conduit::i18n::text_static("settings.appearance.body_font"), chat_font)
                    .description(settings_description(conduit::i18n::text_static("settings.appearance.used_for_messages_and_input_defaults_to")))
                    .disabled(disabled),
                SettingItem::new(conduit::i18n::text_static("settings.appearance.font_size"), chat_size)
                    .description(settings_description(conduit::i18n::text_static("settings.appearance.12_20_px_default_15")))
                    .disabled(disabled),
                SettingItem::new(conduit::i18n::text_static("settings.appearance.line_height"), chat_line_height)
                    .description(settings_description(conduit::i18n::text_static("settings.appearance.1_2_2_0_default_1_5")))
                    .disabled(disabled),
            ]),
            settings_section(conduit::i18n::text_static("settings.appearance.code_blocks")).items(vec![
                SettingItem::new(conduit::i18n::text_static("settings.appearance.code_font"), code_font)
                    .description(settings_description(conduit::i18n::text_static("settings.appearance.used_for_code_blocks_and_tool_output")))
                    .disabled(disabled),
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
            ]),
            settings_section(conduit::i18n::text_static("settings.appearance.chat_preview")).items(vec![
                SettingItem::render(move |_, _, cx| {
                    let app = preview_view.read(cx);
                    chat_markdown_view(
                        "appearance-chat-preview".into(),
                        conduit::i18n::text_static("settings.appearance.change_the_options_above_to_preview_chat"),
                        &app.chat_typography(),
                        "appearance-preview-code",
                        false,
                    )
                })
                .keywords([conduit::i18n::text_static("mermaid.preview"), conduit::i18n::text_static("settings.appearance.chat_preview")]),
                SettingItem::new(conduit::i18n::text_static("settings.appearance.restore_defaults"), reset)
                    .description(settings_description(conduit::i18n::text_static("settings.appearance.reset_chat_font_size_line_height_and")))
                    .disabled(disabled),
            ]),
            settings_section(conduit::i18n::text_static("wallpaper.title")).item(
                SettingItem::render(move |_, _, cx| view.read(cx).background_controls(view.clone()))
                    .keywords([conduit::i18n::text_static("wallpaper.title"), "Wallhaven", "wallpaper", "background"]),
            ),
        ])
    }
}
