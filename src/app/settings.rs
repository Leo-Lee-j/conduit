//! Settings dialogs, preferences, and CLI configuration.

use super::*;

pub(super) fn settings_label(text: &'static str) -> impl IntoElement {
    div()
        .w(px(84.))
        .flex_shrink_0()
        .text_size(px(11.))
        .text_color(rgb(palette().faint))
        .child(text)
}

/// A read-only settings card row: fixed label on the left, value on the right.
pub(super) fn settings_row(label: &'static str, value: String, divider: bool) -> AnyElement {
    let mut row = h_flex().px_4().py_3().gap_4().items_center();
    if divider {
        row = row.border_t_1().border_color(rgb(palette().border));
    }
    row.child(settings_label(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .child(value),
        )
        .into_any_element()
}

/// A bordered settings card: a title row followed by the caller's own rows.
pub(super) fn settings_card(title: &'static str, trailing: AnyElement) -> Div {
    v_flex()
        .w_full()
        .overflow_hidden()
        .rounded(px(12.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface_strong))
        .child(
            h_flex()
                .px_4()
                .py_3()
                .items_center()
                .border_b_1()
                .border_color(rgb(palette().border))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_semibold()
                        .text_color(rgb(palette().text))
                        .child(title),
                )
                .child(div().flex_1())
                .child(trailing),
        )
}

/// The short right-aligned note a settings card header ends with.
pub(super) fn settings_card_note(text: &'static str) -> AnyElement {
    div()
        .text_size(px(10.))
        .text_color(rgb(palette().faint))
        .child(text)
        .into_any_element()
}

/// A settings row: fixed label, wrapping hint, and a right-aligned control.
/// Unlike [`settings_row`] the control is an element, so pickers and steppers
/// can sit at the trailing edge.
pub(super) fn settings_control_row(
    label: &'static str,
    hint: &'static str,
    control: AnyElement,
    divider: bool,
) -> AnyElement {
    let mut row = h_flex().w_full().px_4().py_3().gap_4().items_center();
    if divider {
        row = row.border_t_1().border_color(rgb(palette().border));
    }
    row.child(settings_label(label))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .child(hint),
        )
        .child(control)
        .into_any_element()
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
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone, Copy)]
pub(super) enum TypographyField {
    ChatSize,
    ChatLineHeight,
    CodeSize,
}

impl TypographyField {
    pub(super) fn range(self) -> (f32, f32) {
        match self {
            Self::ChatSize => (MIN_CHAT_FONT_SIZE, MAX_CHAT_FONT_SIZE),
            Self::ChatLineHeight => (MIN_CHAT_LINE_HEIGHT, MAX_CHAT_LINE_HEIGHT),
            Self::CodeSize => (MIN_CODE_FONT_SIZE, MAX_CODE_FONT_SIZE),
        }
    }
    pub(super) fn stepper_labels(self) -> (&'static str, &'static str) {
        match self {
            Self::ChatSize => ("减小聊天字号", "增大聊天字号"),
            Self::ChatLineHeight => ("减小聊天行高", "增大聊天行高"),
            Self::CodeSize => ("减小代码字号", "增大代码字号"),
        }
    }
    pub(super) fn display_value(self, value: f32) -> String {
        match self {
            Self::ChatLineHeight => format!("{value:.1}"),
            _ => format!("{value:.0}"),
        }
    }
}

pub(super) fn typography_stepper(
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
        .gap_1()
        .items_center()
        .child(
            Button::new(format!("{id}-minus"))
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
                .w(px(32.))
                .justify_center()
                .text_size(px(12.))
                .text_color(rgb(if disabled {
                    palette().faint
                } else {
                    palette().text
                }))
                .child(display),
        )
        .child(
            Button::new(format!("{id}-plus"))
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
#[path = "settings/cli.rs"]
mod cli_settings;
mod general;
#[path = "settings/models.rs"]
mod model_sync;

impl ConduitApp {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_all_models_async(window, cx);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            // Center the dialog vertically. The content follows the viewport
            // height (see settings_modal_content), so the same formula keeps
            // top and bottom margins equal and still fits the minimum window.
            let viewport_height = f32::from(window.viewport_size().height);
            let modal_height = (viewport_height - 96.).clamp(360., 560.);
            let margin_top = ((viewport_height - modal_height) * 0.5).max(24.);
            dialog
                .w(px(840.))
                .margin_top(px(margin_top))
                .p_0()
                .gap(px(0.))
                .rounded(px(16.))
                .bg(rgb(palette().surface))
                .border_color(rgb(palette().border))
                .close_button(true)
                .overlay_closable(true)
                .content(move |content, window, cx| {
                    let body = view.read(cx).settings_modal_content(window, view.clone());
                    content.child(body)
                })
        });
        cx.notify();
    }

    pub(super) fn open_about_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .w(px(440.))
                .margin_top(px(120.))
                .title("关于")
                .content(move |content, _, _| {
                    content.child(
                        v_flex()
                            .gap_4()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(
                                        h_flex()
                                            .w(px(40.))
                                            .h(px(40.))
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .rounded(px(10.))
                                            .bg(rgb(palette().text))
                                            .text_size(px(20.))
                                            .font_bold()
                                            .text_color(rgb(palette().on_text))
                                            .child("C"),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .text_size(px(18.))
                                                    .font_semibold()
                                                    .text_color(rgb(palette().text))
                                                    .child("Conduit"),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .text_color(rgb(palette().faint))
                                                    .child(format!(
                                                        "版本 {} · Windows",
                                                        env!("CARGO_PKG_VERSION")
                                                    )),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(relative(1.6))
                                    .text_color(rgb(palette().muted))
                                    .child(
                                        "Conduit 是本地 CLI 会话工作台：把 Claude Code 与 Codex 的对话、工作区和 Git 变更收进一个界面。所有文件与历史都保留在你自己的机器上，不上传云端。",
                                    ),
                            )
                            .child(
                                h_flex().justify_end().child(
                                    Button::new("about-check-update")
                                        .label("检查更新")
                                        .small()
                                        .secondary()
                                        .cursor_pointer()
                                        .tooltip("更新检查暂未开放"),
                                ),
                            ),
                    )
                })
        });
        cx.notify();
    }

    pub(super) fn settings_modal_content(&self, window: &Window, view: Entity<Self>) -> AnyElement {
        let (title, subtitle) = match self.settings_tab {
            SettingsTab::General => ("通用", "本机数据与诊断设置"),
            SettingsTab::Appearance => ("外观", "主题、聊天排版与侧栏显示密度"),
            SettingsTab::Session => ("会话", "会话命名与标签页行为"),
            SettingsTab::Git => ("Git", "配置 Commit Message 的 AI 生成提示词"),
            SettingsTab::Cli => ("CLI", "可执行文件路径查找顺序与版本检测说明"),
            SettingsTab::Provider(provider) => (provider.label(), "管理可执行文件路径与版本检测"),
            SettingsTab::Models => ("模型同步", "查看 CLI 当前模型与 cc-switch 同步的模型"),
        };
        let nav_item = |tab: SettingsTab, label: &'static str, leading: AnyElement| -> AnyElement {
            let view = view.clone();
            let active = self.settings_tab == tab;
            Button::new(match tab {
                SettingsTab::General => "settings-tab-general",
                SettingsTab::Appearance => "settings-tab-appearance",
                SettingsTab::Session => "settings-tab-session",
                SettingsTab::Git => "settings-tab-git",
                SettingsTab::Cli => "settings-tab-cli",
                SettingsTab::Provider(Provider::Claude) => "settings-tab-claude",
                SettingsTab::Provider(Provider::Codex) => "settings-tab-codex",
                SettingsTab::Models => "settings-tab-models",
            })
            .ghost()
            .h(px(32.))
            .w_full()
            .px_2()
            .rounded(px(8.))
            .selected(active)
            .accessibility_label(label)
            .cursor_pointer()
            .on_click(move |_, _, cx| {
                view.update(cx, |this, cx| {
                    this.settings_tab = tab;
                    cx.notify();
                });
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(leading)
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(rgb(if active {
                                palette().text
                            } else {
                                palette().muted
                            }))
                            .child(label),
                    ),
            )
            .into_any_element()
        };
        let section = |text: &'static str| {
            div()
                .px_2()
                .pt_3()
                .pb_1()
                .text_size(px(10.))
                .text_color(rgb(palette().faint))
                .child(text)
        };
        let general_icon = if self.settings_tab == SettingsTab::General {
            palette().text
        } else {
            palette().faint
        };
        let general_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Settings)
                    .small()
                    .text_color(rgb(general_icon)),
            )
            .into_any_element();
        let appearance_icon = if self.settings_tab == SettingsTab::Appearance {
            palette().text
        } else {
            palette().faint
        };
        let appearance_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Palette)
                    .small()
                    .text_color(rgb(appearance_icon)),
            )
            .into_any_element();
        let models_icon = if self.settings_tab == SettingsTab::Models {
            palette().text
        } else {
            palette().faint
        };
        let models_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(IconName::Cpu)
                    .small()
                    .text_color(rgb(models_icon)),
            )
            .into_any_element();
        let cli_icon = if self.settings_tab == SettingsTab::Cli {
            palette().text
        } else {
            palette().faint
        };
        let cli_leading = h_flex()
            .w(px(14.))
            .flex_shrink_0()
            .justify_center()
            .child(
                Icon::new(gpui_kit::assets::IconName::Terminal)
                    .small()
                    .text_color(rgb(cli_icon)),
            )
            .into_any_element();
        let nav = v_flex()
            .w(px(208.))
            .h_full()
            .p_3()
            .gap_1()
            .bg(rgb(palette().sidebar_bg))
            .border_r_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .px_2()
                    .pt_2()
                    .pb_3()
                    .text_size(px(16.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child("设置"),
            )
            .child(section("应用"))
            .child(nav_item(SettingsTab::General, "通用", general_leading))
            .child(nav_item(
                SettingsTab::Appearance,
                "外观",
                appearance_leading,
            ))
            .child(nav_item(
                SettingsTab::Session,
                "会话",
                Icon::new(gpui_kit::assets::IconName::Sparkles)
                    .small()
                    .text_color(rgb(if self.settings_tab == SettingsTab::Session {
                        palette().text
                    } else {
                        palette().faint
                    }))
                    .into_any_element(),
            ))
            .child(nav_item(
                SettingsTab::Git,
                "Git",
                Icon::new(gpui_kit::assets::IconName::GitFork)
                    .small()
                    .text_color(rgb(if self.settings_tab == SettingsTab::Git {
                        palette().text
                    } else {
                        palette().faint
                    }))
                    .into_any_element(),
            ))
            .child(section("CLI"))
            .child(nav_item(SettingsTab::Cli, "CLI", cli_leading))
            .child(nav_item(
                SettingsTab::Provider(Provider::Claude),
                "Claude Code",
                provider_icon(Provider::Claude).into_any_element(),
            ))
            .child(nav_item(
                SettingsTab::Provider(Provider::Codex),
                "Codex",
                provider_icon(Provider::Codex).into_any_element(),
            ))
            .child(section("模型"))
            .child(nav_item(SettingsTab::Models, "模型同步", models_leading))
            .child(div().flex_1())
            .child(
                div()
                    .px_2()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
                    .child("本地 CLI · Windows"),
            );
        let page = match self.settings_tab {
            SettingsTab::General => self.settings_general_content(view.clone()),
            SettingsTab::Appearance => self.settings_appearance_content(view.clone()),
            SettingsTab::Session => self.settings_session_content(view.clone()),
            SettingsTab::Git => self.settings_git_content(view.clone()),
            SettingsTab::Cli => self.settings_cli_content(),
            SettingsTab::Provider(provider) => {
                self.settings_provider_content(provider, view.clone())
            }
            SettingsTab::Models => self.settings_models_content(view.clone()),
        };
        let body = v_flex()
            .id("settings-modal-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .py_5()
            .gap_4()
            .when(!self.writable, |column| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(palette().danger_bg))
                        .text_size(px(11.))
                        .text_color(rgb(palette().danger_text))
                        .child("当前数据目录为只读，无法保存设置或会话模型。"),
                )
            })
            .when_some(self.message.clone(), |column, message| {
                column.child(
                    div()
                        .px_3()
                        .py_2()
                        .rounded(px(8.))
                        .bg(rgb(palette().danger_bg))
                        .text_size(px(11.))
                        .text_color(rgb(palette().danger_text))
                        .child(message),
                )
            })
            .child(page);
        let right = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(
                v_flex()
                    .px_6()
                    .pt_5()
                    .pb_4()
                    .pr(px(44.))
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_semibold()
                            .text_color(rgb(palette().text))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(subtitle),
                    ),
            )
            .child(div().h(px(1.)).bg(rgb(palette().border)))
            .child(body)
            .child(
                h_flex()
                    .px_6()
                    .py_3()
                    .gap_3()
                    .items_center()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
                            .child("配置保存在本地，具体生效范围见各页面说明"),
                    )
                    .child(
                        Button::new("settings-done")
                            .label("完成")
                            .small()
                            .primary()
                            .cursor_pointer()
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            );
        // Keep the footer reachable at the documented minimum window size
        // (850x500 client area). The dialog layer clips content taller than the
        // viewport minus its edge margins, so the height follows the window and
        // the body scrolls instead of losing the 完成 button.
        let modal_height = (f32::from(window.viewport_size().height) - 96.).clamp(360., 560.);
        h_flex()
            .w_full()
            .h(px(modal_height))
            .items_stretch()
            .overflow_hidden()
            .rounded(px(16.))
            .child(nav)
            .child(right)
            .into_any_element()
    }
}
