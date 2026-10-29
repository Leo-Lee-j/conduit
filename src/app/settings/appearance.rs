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
    v_flex()
        .w_full()
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
        ))
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
        let sessions = self.store.ui.sessions_per_workspace.clamp(1, 30);
        let sessions_limit = count_stepper_field(
            "sessions-limit",
            sessions,
            1,
            30,
            "减少工作区会话数",
            "增加工作区会话数",
            disabled,
            view.clone(),
            |app, value| app.store.ui.sessions_per_workspace = value,
        );
        let max_tabs = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        let max_tabs_limit = count_stepper_field(
            "max-tabs",
            max_tabs,
            1,
            MAX_TAB_LIMIT,
            "减少最大标签数量",
            "增加最大标签数量",
            disabled,
            view.clone(),
            |app, value| app.store.ui.max_tabs = value,
        );
        let full_width = switch_field(
            view.clone(),
            |app| app.store.ui.full_width_content,
            |app, value| app.store.ui.full_width_content = value,
        );
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
            SettingGroup::new()
                .title("主题")
                .item(
                    SettingItem::render(move |_, _, _| {
                        theme_options(selected, theme_view.clone(), disabled)
                    })
                    .keywords(["主题", "浅色", "深色", "跟随系统"]),
                )
                .footer(|_, _| "主题切换会立即应用并保存；跟随系统时，Windows 明暗变化会自动同步。"),
            SettingGroup::new().title("聊天文字").items(vec![
                SettingItem::new("正文字体", chat_font)
                    .description("用户消息、回复与输入框；默认跟随系统界面字体")
                    .disabled(disabled),
                SettingItem::new("字号", chat_size)
                    .description("12–20 px；默认 15")
                    .disabled(disabled),
                SettingItem::new("行高", chat_line_height)
                    .description("1.2–2.0；默认 1.5")
                    .disabled(disabled),
            ]),
            SettingGroup::new().title("代码块").items(vec![
                SettingItem::new("代码字体", code_font)
                    .description("Markdown 代码块、行内代码与工具输出；默认系统等宽字体")
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
            SettingGroup::new().title("聊天预览").items(vec![
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
            SettingGroup::new().title("内容区域").item(
                SettingItem::new("内容铺满", full_width)
                    .description("关闭时右侧内容以 840px 居中显示；开启后铺满整个内容区")
                    .disabled(disabled),
            ),
            SettingGroup::new().title("侧栏").item(
                SettingItem::new("工作区会话数", sessions_limit)
                    .description("默认 5 条，范围 1–30，超出时侧栏可展开")
                    .disabled(disabled),
            ),
            SettingGroup::new().title("标签页").item(
                SettingItem::new("最大 tab 数量", max_tabs_limit)
                    .description("默认 8 个，范围 1–20；超过数量自动关闭最老的标签")
                    .disabled(disabled),
            ),
        ])
    }
}
