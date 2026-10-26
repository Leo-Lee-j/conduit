//! Appearance settings.

use super::*;

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
        for select in [&self.chat_font_select, &self.code_font_select] {
            select.update(cx, |state, cx| {
                state.set_selected_index(Some(IndexPath::new(0)), window, cx);
            });
        }
        self.apply_code_font(cx);
        self.persist();
        cx.notify();
    }

    pub(in crate::app) fn settings_appearance_content(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let selected = self.store.ui.appearance;
        let theme_option = |mode: AppearanceMode,
                            label: &'static str,
                            detail: &'static str,
                            icon: gpui_kit::assets::IconName,
                            id: &'static str|
         -> AnyElement {
            let view = view.clone();
            let active = selected == mode;
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
            Button::new(id)
                .ghost()
                .h(px(54.))
                .w_full()
                .px_3()
                .rounded(px(8.))
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
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_3()
                        .child(Icon::new(icon).small().text_color(rgb(icon_color)))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .font_medium()
                                        .text_color(rgb(label_color))
                                        .child(label),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().faint))
                                        .child(detail),
                                ),
                        )
                        .when(active, |row| {
                            row.child(
                                Icon::new(gpui_kit::assets::IconName::Check)
                                    .small()
                                    .text_color(rgb(check_color)),
                            )
                        }),
                )
                .into_any_element()
        };
        let value = self.store.ui.sessions_per_workspace.clamp(1, 30);
        let decrease = view.clone();
        let increase = view.clone();
        let max_tabs = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        let max_tabs_limit = MAX_TAB_LIMIT;
        let tabs_decrease = view.clone();
        let tabs_increase = view.clone();
        v_flex()
            .w_full()
            .gap_4()
            .child(
                settings_card("主题", settings_card_note("立即生效"))
                    .child(
                        v_flex()
                            .w_full()
                            .p_2()
                            .gap_1()
                            .child(theme_option(
                                AppearanceMode::System,
                                "跟随系统",
                                "随 Windows 的浅色 / 深色外观自动切换",
                                gpui_kit::assets::IconName::Monitor,
                                "appearance-theme-system",
                            ))
                            .child(theme_option(
                                AppearanceMode::Light,
                                "浅色",
                                "始终使用浅色背景与深色文字",
                                gpui_kit::assets::IconName::Sun,
                                "appearance-theme-light",
                            ))
                            .child(theme_option(
                                AppearanceMode::Dark,
                                "深色",
                                "始终使用深色背景与浅色文字",
                                gpui_kit::assets::IconName::Moon,
                                "appearance-theme-dark",
                            )),
                    ),
            )
            .child({
                let chat = self.chat_typography();
                let font_size = chat.body_size;
                let line_height = chat.line_height;
                settings_card("聊天文字", settings_card_note("立即生效"))
                    .child(settings_control_row(
                        "正文字体",
                        "用户消息、回复与输入框；默认跟随系统界面字体",
                        div()
                            .w(px(188.))
                            .flex_shrink_0()
                            .child(
                                Select::new(&self.chat_font_select)
                                    .with_size(Size::Small)
                                    .w_full()
                                    .menu_width(px(260.))
                                    .menu_max_h(px(320.))
                                    .search_placeholder("搜索字体")
                                    .accessibility_label("聊天正文字体")
                                    .disabled(disabled),
                            )
                            .into_any_element(),
                        false,
                    ))
                    .child(settings_control_row(
                        "字号",
                        "12–20 px；默认 15",
                        typography_stepper(
                            "chat-font-size",
                            font_size,
                            disabled,
                            TypographyField::ChatSize,
                            view.clone(),
                        ),
                        true,
                    ))
                    .child(settings_control_row(
                        "行高",
                        "1.2–2.0；默认 1.5",
                        typography_stepper(
                            "chat-line-height",
                            line_height,
                            disabled,
                            TypographyField::ChatLineHeight,
                            view.clone(),
                        ),
                        true,
                    ))
            })
            .child({
                let chat = self.chat_typography();
                let code_size = chat.code_size;
                let copy_view = view.clone();
                let highlight_view = view.clone();
                settings_card("代码块", settings_card_note("立即生效"))
                    .child(settings_control_row(
                        "代码字体",
                        "Markdown 代码块、行内代码与工具输出；默认系统等宽字体",
                        div()
                            .w(px(188.))
                            .flex_shrink_0()
                            .child(
                                Select::new(&self.code_font_select)
                                    .with_size(Size::Small)
                                    .w_full()
                                    .menu_width(px(260.))
                                    .menu_max_h(px(320.))
                                    .search_placeholder("搜索字体")
                                    .accessibility_label("代码字体")
                                    .disabled(disabled),
                            )
                            .into_any_element(),
                        false,
                    ))
                    .child(settings_control_row(
                        "代码字号",
                        "10–18 px；默认 13",
                        typography_stepper(
                            "code-font-size",
                            code_size,
                            disabled,
                            TypographyField::CodeSize,
                            view.clone(),
                        ),
                        true,
                    ))
                    .child(settings_control_row(
                        "复制按钮",
                        "在每个代码块右上角显示复制按钮",
                        Switch::new("code-copy-setting")
                            .checked(self.store.ui.code_copy_button)
                            .disabled(disabled)
                            .on_change(move |checked, _, cx| {
                                copy_view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.code_copy_button = *checked;
                                    this.persist();
                                    cx.notify();
                                });
                            })
                            .into_any_element(),
                        true,
                    ))
                    .child(settings_control_row(
                        "语法高亮",
                        "按语言着色；关闭后保持等宽单色",
                        Switch::new("code-syntax-setting")
                            .checked(self.store.ui.code_syntax_highlight)
                            .disabled(disabled)
                            .on_change(move |checked, _, cx| {
                                highlight_view.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.code_syntax_highlight = *checked;
                                    this.persist();
                                    cx.notify();
                                });
                            })
                            .into_any_element(),
                        true,
                    ))
            })
            .child({
                let chat = self.chat_typography();
                let reset_view = view.clone();
                let preview = chat_markdown_view(
                    "appearance-chat-preview".into(),
                    "调整上方选项可即时预览聊天正文。\n\n```rust\nfn main() {\n    println!(\"hello, conduit\");\n}\n```\n\n行内 `code` 与代码块都会跟随等宽字体设置。",
                    &chat,
                    "appearance-preview-code",
                    false,
                );
                settings_card(
                    "聊天预览",
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
                        .into_any_element(),
                )
                .child(div().px_4().py_3().child(preview))
            })
            .child({
                let toggle = view.clone();
                let full_width = self.store.ui.full_width_content;
                settings_card("内容区域", settings_card_note("立即生效"))
                    .child(settings_control_row(
                        "内容铺满",
                        "关闭时右侧内容以 840px 居中显示；开启后铺满整个内容区",
                        Switch::new("full-width-content-setting")
                            .checked(full_width)
                            .disabled(disabled)
                            .on_change(move |checked, _, cx| {
                                toggle.update(cx, |this, cx| {
                                    if !this.writable || this.closing {
                                        return;
                                    }
                                    this.store.ui.full_width_content = *checked;
                                    this.persist();
                                    cx.notify();
                                });
                            })
                            .into_any_element(),
                        false,
                    ))
            })
            .child(
                settings_card("侧栏", div().into_any_element())
                    .child(settings_control_row(
                        "工作区会话数",
                        "默认 5 条，范围 1–30，超出时侧栏可展开",
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("sessions-limit-minus")
                                    .label("−")
                                    .accessibility_label("减少工作区会话数")
                                    .small()
                                    .ghost()
                                    .disabled(disabled || value <= 1)
                                    .when(!disabled && value > 1, |button| button.cursor_pointer())
                                    .on_click(move |_, _, cx| {
                                        decrease.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.sessions_per_workspace = this
                                                .store
                                                .ui
                                                .sessions_per_workspace
                                                .clamp(1, 30)
                                                .saturating_sub(1)
                                                .max(1);
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                h_flex()
                                    .w(px(26.))
                                    .justify_center()
                                    .text_size(px(12.))
                                    .text_color(rgb(if disabled {
                                        palette().faint
                                    } else {
                                        palette().text
                                    }))
                                    .child(value.to_string()),
                            )
                            .child(
                                Button::new("sessions-limit-plus")
                                    .label("+")
                                    .accessibility_label("增加工作区会话数")
                                    .small()
                                    .ghost()
                                    .disabled(disabled || value >= 30)
                                    .when(!disabled && value < 30, |button| button.cursor_pointer())
                                    .on_click(move |_, _, cx| {
                                        increase.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.sessions_per_workspace = (this
                                                .store
                                                .ui
                                                .sessions_per_workspace
                                                .clamp(1, 30)
                                                + 1)
                                                .min(30);
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                            .into_any_element(),
                        false,
                    )),
            )
            .child(
                settings_card("标签页", div().into_any_element()).child(
                    settings_control_row(
                        "最大 tab 数量",
                        "默认 8 个，范围 1–20；超过数量自动关闭最老的标签",
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Button::new("max-tabs-minus")
                                    .label("−")
                                    .accessibility_label("减少最大标签数量")
                                    .small()
                                    .ghost()
                                    .disabled(disabled || max_tabs <= 1)
                                    .when(!disabled && max_tabs > 1, |button| button.cursor_pointer())
                                    .on_click(move |_, _, cx| {
                                        tabs_decrease.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.max_tabs = this
                                                .store
                                                .ui
                                                .max_tabs
                                                .clamp(1, max_tabs_limit)
                                                .saturating_sub(1)
                                                .max(1);
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                h_flex()
                                    .w(px(26.))
                                    .justify_center()
                                    .text_size(px(12.))
                                    .text_color(rgb(if disabled {
                                        palette().faint
                                    } else {
                                        palette().text
                                    }))
                                    .child(max_tabs.to_string()),
                            )
                            .child(
                                Button::new("max-tabs-plus")
                                    .label("+")
                                    .accessibility_label("增加最大标签数量")
                                    .small()
                                    .ghost()
                                    .disabled(disabled || max_tabs >= max_tabs_limit)
                                    .when(
                                        !disabled && max_tabs < max_tabs_limit,
                                        |button| button.cursor_pointer(),
                                    )
                                    .on_click(move |_, _, cx| {
                                        tabs_increase.update(cx, |this, cx| {
                                            if !this.writable || this.closing {
                                                return;
                                            }
                                            this.store.ui.max_tabs = (this
                                                .store
                                                .ui
                                                .max_tabs
                                                .clamp(1, max_tabs_limit)
                                                + 1)
                                                .min(max_tabs_limit);
                                            this.persist();
                                            cx.notify();
                                        });
                                    }),
                            )
                            .into_any_element(),
                        false,
                    ),
                ),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .line_height(relative(1.5))
                    .text_color(rgb(palette().faint))
                    .child("主题切换会立即应用并保存；跟随系统时，Windows 明暗变化会自动同步。"),
            )
            .into_any_element()
    }
}
