//! Settings dialogs, preferences, and CLI configuration.

use super::*;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};

/// One settings page: title and icon. The page header is collapsed, so the
/// title only appears in the sidebar.
fn settings_page(title: impl Into<SharedString>, icon: FullIconName) -> SettingPage {
    SettingPage::new(title)
        .icon(icon)
        .resettable(false)
        .header_style(&page_header_style())
}

/// SettingPage always renders its own header; the design has no page title bar,
/// so the header is collapsed to zero height and clipped.
fn page_header_style() -> StyleRefinement {
    StyleRefinement::default()
        .h(px(0.))
        .p_0()
        .border_0()
        .overflow_hidden()
}

fn sidebar_style() -> StyleRefinement {
    StyleRefinement::default()
        .bg(rgb(palette().sidebar_bg))
        .border_r_1()
        .border_color(rgb(palette().border))
}

/// A read-only setting value, right-aligned in the row.
pub(super) fn value_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        div()
            .max_w(px(240.))
            .truncate()
            .text_size(px(11.))
            .text_color(rgb(palette().muted))
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
            .text_size(px(11.))
            .line_height(relative(1.5))
            .text_color(rgb(palette().faint))
            .child(value.clone())
    })
}

/// A wrapping paragraph for errors, in the danger color.
pub(super) fn error_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        div()
            .w_full()
            .text_size(px(11.))
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
            .gap_1()
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
        self.reload_all_models_async(false, window, cx);
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
                // The library paints its scrim at full opacity on the first
                // frame and fades only the surface, so an entrance animation
                // reads as the panel rising out from behind the scrim.
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .overlay(true)
                .overlay_closable(true)
                .w(px(840.))
                .margin_top(px(margin_top))
                .p_0()
                .gap(px(0.))
                .rounded(px(16.))
                .bg(rgb(palette().surface))
                .border_color(rgb(palette().border))
                .close_button(false)
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
                                    .child(app_icon(40.))
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
                                        "Conduit 是本地 CLI 会话工作台：把 Claude Code、Codex 与 OpenCode 的对话、工作区和 Git 变更收进一个界面。所有文件与历史都保留在你自己的机器上，不上传云端。",
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
        let disabled = !self.writable || self.closing;
        let settings = Settings::new("conduit-settings")
            .sidebar_width(px(208.))
            .sidebar_size_range(px(184.)..px(240.))
            .with_group_variant(GroupBoxVariant::Outline)
            .sidebar_style(&sidebar_style())
            .pages(vec![
                self.settings_general_page(disabled, view.clone()),
                self.settings_appearance_page(disabled, view.clone()),
                self.settings_session_page(disabled, view.clone()),
                self.settings_git_page(disabled, view.clone()),
                self.settings_cli_page(disabled, view.clone()),
                self.settings_models_page(disabled, view.clone()),
            ]);
        // The dialog layer clips content taller than the viewport minus its edge
        // margins, so the content follows the window and each page scrolls.
        let modal_height = (f32::from(window.viewport_size().height) - 96.).clamp(360., 560.);
        v_flex()
            .w_full()
            .h(px(modal_height))
            .overflow_hidden()
            .rounded(px(16.))
            .child(div().flex_1().min_h_0().w_full().child(settings))
            .into_any_element()
    }
}
