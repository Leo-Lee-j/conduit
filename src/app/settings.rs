//! Settings dialogs, preferences, and CLI configuration.

use super::*;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};

/// Each page keeps its title visible while the settings below it scroll.
fn settings_page(title: &'static str, icon: FullIconName) -> SettingPage {
    let description = match title {
        "通用" => "调整工作台布局。",
        "外观" => "选择主题，调整聊天文字与代码的阅读体验。",
        "自动命名" => "设置会话标题的生成方式与使用的模型。",
        "Git 提交" => "自定义提交说明的生成模型与提示词。",
        "AI 引擎" => "选择引擎，检查本机 CLI 的启动路径与版本。",
        "模型与服务" => "管理服务端点和模型目录；新会话默认值单独设置。",
        "新会话默认" => "按引擎设置新会话使用的服务和模型。现有会话保留自己的选择。",
        "数据与更新" => "管理本机数据、更新偏好与诊断。",
        _ => "",
    };
    SettingPage::new(title)
        .icon(icon)
        .resettable(false)
        .description(description)
        .header_style(&page_header_style())
}

fn page_header_style() -> StyleRefinement {
    StyleRefinement::default()
        .px(px(24.))
        .py(px(10.))
        .gap(px(6.))
        .flex_shrink_0()
        .text_size(px(18.))
        .font_semibold()
        .text_color(rgb(palette().text))
        .bg(rgb(palette().surface))
        .border_color(rgb(palette().border))
}

/// Section headings live in the card. Leaving the library's group title unset
/// keeps the sidebar flat instead of duplicating every section as a submenu.
fn settings_section(title: impl Into<SharedString>) -> SettingGroup {
    let title = title.into();
    let keywords = [title.clone()];
    SettingGroup::new().py(px(12.)).gap(px(10.)).item(
        SettingItem::render(move |_, _, _| {
            div()
                .w_full()
                .pb(px(12.))
                .border_b_1()
                .border_color(rgb(palette().border))
                .text_size(px(13.))
                .font_semibold()
                .text_color(rgb(palette().text))
                .child(title.clone())
        })
        .keywords(keywords),
    )
}

fn settings_modal_height(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) - 64.).clamp(0., 680.)
}

/// A read-only setting value, right-aligned in the row.
pub(super) fn value_field(value: String) -> SettingField<SharedString> {
    SettingField::render(move |_, _, _| {
        let tooltip_value = value.clone();
        div()
            .max_w(px(320.))
            .truncate()
            .text_size(px(12.))
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
            .text_size(px(12.))
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
            .text_size(px(12.))
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
mod config_editor;
#[path = "settings/connections.rs"]
mod connections;
mod defaults;
mod general;
#[path = "settings/models.rs"]
mod model_sync;
pub(super) use connections::ConnectionsUi;
#[path = "settings/extensions.rs"]
mod extensions_view;
#[path = "settings/navigation.rs"]
mod navigation;
pub(super) use extensions_view::ExtensionsUi;
pub(super) use navigation::SettingsUi;

impl ConduitApp {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reload_all_models_async(false, window, cx);
        let view = cx.entity();
        if matches!(self.settings_ui.page, 4 | 9) {
            self.ensure_extensions(window, cx, false);
        }
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let close_view = view.clone();
            let viewport_height = f32::from(window.viewport_size().height);
            let modal_height = settings_modal_height(window);
            let margin_top = ((viewport_height - modal_height) * 0.5).max(16.);
            let modal_width = (f32::from(window.viewport_size().width) - 48.).clamp(0., 1080.);
            dialog
                // The library paints its scrim at full opacity on the first
                // frame and fades only the surface, so an entrance animation
                // reads as the panel rising out from behind the scrim.
                .entrance(gpui_kit::component::dialog::DialogEntrance::None)
                .overlay(true)
                .overlay_closable(true)
                .w(px(modal_width))
                .margin_top(px(margin_top))
                .p_0()
                .gap(px(0.))
                .rounded(px(16.))
                .bg(rgb(palette().surface))
                .border_color(rgb(palette().border))
                .close_button(false)
                .on_close(move |_, _, cx| {
                    close_view.update(cx, |app, cx| {
                        app.extensions_ui.probe = None;
                        app.extensions_ui.connection_probe = None;
                        cx.notify();
                    });
                })
                .content(move |content, window, cx| {
                    let body = view
                        .read(cx)
                        .settings_modal_content(window, view.clone(), cx);
                    content.child(body)
                })
        });
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
                .title("关于")
                .content(move |content, _, cx| {
                    content.child(
                        v_flex()
                            .id("about-update-body")
                            .max_h(px(height))
                            .overflow_y_scroll()
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
                            .child(view.read(cx).update_content(view.clone())),
                    )
                })
        });
        cx.notify();
    }

    pub(super) fn settings_modal_content(
        &self,
        window: &Window,
        view: Entity<Self>,
        cx: &App,
    ) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let settings = self.settings_body(window, view.clone(), cx);
        let modal_height = settings_modal_height(window);
        v_flex()
            .id("settings-modal")
            .debug_selector(|| "settings-modal".into())
            .w_full()
            .h(px(modal_height))
            .overflow_hidden()
            .rounded(px(16.))
            .child(
                h_flex()
                    .id("settings-modal-header")
                    .debug_selector(|| "settings-modal-header".into())
                    .h(px(56.))
                    .flex_shrink_0()
                    .px(px(20.))
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Icon::new(FullIconName::Settings)
                                    .small()
                                    .text_color(rgb(palette().muted)),
                            )
                            .child(div().text_size(px(14.)).font_semibold().child("设置")),
                    )
                    .child(
                        div()
                            .id("settings-close-control")
                            .debug_selector(|| "settings-close".into())
                            .child(
                                Button::new("settings-close")
                                    .icon(FullIconName::X)
                                    .ghost()
                                    .small()
                                    .accessibility_label("关闭设置")
                                    .tooltip("关闭设置（Esc）")
                                    .cursor_pointer()
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("settings-modal-body")
                    .debug_selector(|| "settings-modal-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(settings),
            )
            .child(
                h_flex()
                    .id("settings-modal-footer")
                    .debug_selector(|| "settings-modal-footer".into())
                    .h(px(36.))
                    .flex_shrink_0()
                    .px(px(20.))
                    .gap_2()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface_strong))
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child(
                        Icon::new(if disabled {
                            FullIconName::Lock
                        } else {
                            FullIconName::Check
                        })
                        .small(),
                    )
                    .child(if let Some(error) = &self.persistence.error {
                        format!("保存失败：{error}")
                    } else if disabled {
                        "当前为只读模式".to_string()
                    } else if self.persistence.busy() {
                        "正在保存偏好设置…".to_string()
                    } else if matches!(self.settings_ui.page, 4 | 9) {
                        "原生配置通过编辑器保存 · 下次启动会话时加载".to_string()
                    } else {
                        "偏好设置已保存 · 编辑表单需点击保存".to_string()
                    }),
            )
            .into_any_element()
    }

    fn settings_shortcuts_page(&self) -> SettingPage {
        let rows = [
            ("最近会话", "ctrl-e"),
            ("搜索会话", "ctrl-p"),
            ("打开设置", "ctrl-m"),
            ("文件名搜索 Shift ×2", "shift"),
            ("切换 Git 侧栏", "ctrl-k"),
        ];
        let items: Vec<_> = rows
            .into_iter()
            .map(|(label, keys)| {
                let field = SettingField::render(move |_, _, _| {
                    gpui_kit::component::kbd::Kbd::new(
                        Keystroke::parse(keys).expect("static shortcut is valid"),
                    )
                    .outline()
                });
                SettingItem::new(label, field)
            })
            .collect();
        settings_page("快捷键", FullIconName::Keyboard)
            .group(settings_section("全局快捷键").items(items))
    }
}
