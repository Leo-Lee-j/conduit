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
                    .h(px(124.))
                    .w_full()
                    .p(px(10.))
                    .rounded(px(10.))
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
                                    .rounded(px(5.))
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(rgb(preview.border))
                                    .bg(rgb(preview.canvas))
                                    .child(
                                        div()
                                            .w(px(22.))
                                            .h_full()
                                            .bg(rgb(preview.sidebar_bg))
                                            .border_r_1()
                                            .border_color(rgb(preview.border)),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .px_2()
                                            .gap(px(5.))
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
                                    .gap(px(6.))
                                    .child(Icon::new(icon).small().text_color(rgb(icon_color)))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(px(13.))
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
                                    .text_size(px(11.))
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
            "跟随系统",
            "自动匹配系统外观",
            gpui_kit::assets::IconName::Monitor,
            "appearance-theme-system",
        )))
        .child(div().flex_1().min_w_0().child(theme_option(
            AppearanceMode::Light,
            "浅色",
            "明亮、清晰",
            gpui_kit::assets::IconName::Sun,
            "appearance-theme-light",
        )))
        .child(div().flex_1().min_w_0().child(theme_option(
            AppearanceMode::Dark,
            "深色",
            "柔和的深色背景",
            gpui_kit::assets::IconName::Moon,
            "appearance-theme-dark",
        )))
        .into_any_element()
}

fn font_select_field(
    view: Entity<ConduitApp>,
    code: bool,
    disabled: bool,
) -> SettingField<SharedString> {
    SettingField::render(move |_, _, cx| {
        let app = view.read(cx);
        let select = if code {
            &app.code_font_select
        } else {
            &app.chat_font_select
        };
        div()
            .w(px(188.))
            .flex_shrink_0()
            .child(
                Select::new(select)
                    .with_size(Size::Small)
                    .w_full()
                    .menu_width(px(260.))
                    .menu_max_h(px(320.))
                    .search_placeholder("搜索字体")
                    .accessibility_label(if code {
                        "代码字体"
                    } else {
                        "聊天正文字体"
                    })
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
        // Loading a theme can reset the monospace pair; reinstall the chat's
        // chosen code font after every mode change.
        self.apply_code_font(cx);
        cx.notify();
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
            body_font: self
                .store
                .ui
                .chat_font_family
                .as_deref()
                .map(SharedString::from),
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
        let chat_font = font_select_field(view.clone(), false, disabled);
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
        let code_font = font_select_field(view.clone(), true, disabled);
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
                .label("恢复默认")
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
            view,
            |app| app.store.ui.code_syntax_highlight,
            |app, value| app.store.ui.code_syntax_highlight = value,
        );
        settings_page("外观", FullIconName::Palette).groups(vec![
            settings_section("主题")
                .item(
                    SettingItem::render(move |_, _, _| {
                        theme_options(selected, theme_view.clone(), disabled)
                    })
                    .keywords(["主题", "浅色", "深色", "跟随系统"]),
                )
                .footer(|_, _| "选择后立即应用；跟随系统时自动同步 Windows 外观。"),
            settings_section("聊天文字").items(vec![
                SettingItem::new("正文字体", chat_font)
                    .description("用于消息与输入框，默认使用系统字体")
                    .disabled(disabled),
                SettingItem::new("字号", chat_size)
                    .description("12–20 px；默认 15")
                    .disabled(disabled),
                SettingItem::new("行高", chat_line_height)
                    .description("1.2–2.0；默认 1.5")
                    .disabled(disabled),
            ]),
            settings_section("代码块").items(vec![
                SettingItem::new("代码字体", code_font)
                    .description("用于代码块与工具输出，默认使用系统等宽字体")
                    .disabled(disabled),
                SettingItem::new("代码字号", code_size)
                    .description("10–18 px；默认 13")
                    .disabled(disabled),
                SettingItem::new("复制按钮", code_copy)
                    .description("在每个代码块右上角显示复制按钮")
                    .disabled(disabled),
                SettingItem::new("行号", code_line_numbers)
                    .description("在代码块左侧显示行号；不随复制内容带出")
                    .disabled(disabled),
                SettingItem::new("语法高亮", code_syntax)
                    .description("按语言着色；关闭后保持等宽单色")
                    .disabled(disabled),
            ]),
            settings_section("聊天预览").items(vec![
                SettingItem::render(move |_, _, cx| {
                    let app = preview_view.read(cx);
                    chat_markdown_view(
                        "appearance-chat-preview".into(),
                        "调整上方选项可即时预览聊天正文。\n\n```rust\nfn main() {\n    println!(\"hello, conduit\");\n}\n```\n\n行内 `code` 与代码块都会跟随等宽字体设置。",
                        &app.chat_typography(),
                        "appearance-preview-code",
                        false,
                    )
                })
                .keywords(["预览", "聊天预览"]),
                SettingItem::new("恢复默认", reset)
                    .description("字体、字号、行高与代码块选项一起恢复默认。")
                    .disabled(disabled),
            ]),
        ])
    }
}
