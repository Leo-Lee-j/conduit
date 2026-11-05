//! A retained settings destination. Forms may open dialogs above it.
use super::*;

impl ConduitApp {
    pub(in crate::app) fn render_settings_workspace(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title_bar = TitleBar::new()
            .bg(rgb(palette().sidebar_bg))
            .border_color(rgb(palette().border))
            .child(
                h_flex().gap_2().child(app_icon(20.)).child(
                    div()
                        .text_size(px(13.))
                        .font_semibold()
                        .child("Conduit · 设置"),
                ),
            );
        v_flex()
            .id("settings-workspace")
            .debug_selector(|| "settings-workspace".into())
            .track_focus(&self.settings_ui.focus)
            .size_full()
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .child(title_bar)
            .child(
                h_flex()
                    .id("settings-workspace-header")
                    .debug_selector(|| "settings-workspace-header".into())
                    .h(px(64.))
                    .flex_shrink_0()
                    .px_4()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new("settings-back")
                            .debug_selector(|| "settings-close".into())
                            .icon(FullIconName::ArrowLeft)
                            .label("返回工作台")
                            .ghost()
                            .tooltip("返回工作台（Esc）")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                            ),
                    )
                    .child(
                        div().flex_1().min_w_0().max_w(px(520.)).child(
                            Input::new(&self.settings_ui.search)
                                .prefix(FullIconName::Search)
                                .aria_label("搜索全部设置")
                                .cleanable(true),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child("Ctrl + F"),
                    ),
            )
            .child(
                div()
                    .id("settings-workspace-body")
                    .debug_selector(|| "settings-workspace-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.settings_body(window, cx.entity(), cx)),
            )
            .child(self.settings_save_status())
            .on_action(cx.listener(|this, _: &OpenSettingsShortcut, window, cx| {
                this.settings_ui
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                if event.keystroke.modifiers.control && event.keystroke.key == "f" {
                    this.settings_ui
                        .search
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "escape" {
                    this.settings_escape(window, cx);
                    cx.stop_propagation();
                } else if event.keystroke.key == "enter"
                    && this
                        .settings_ui
                        .search
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                {
                    let query = this.settings_ui.search.read(cx).value();
                    if let Some(result) = this.settings_search_matches(&query).into_iter().next() {
                        this.open_settings_result(result, window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .on_action(
                cx.listener(|this, _: &gpui_kit::component::input::Escape, window, cx| {
                    if !window.has_active_dialog(cx) {
                        this.settings_escape(window, cx);
                    }
                }),
            )
            .into_any_element()
    }

    fn settings_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_ui.search.read(cx).value().is_empty() {
            self.close_settings(window, cx);
        } else {
            self.settings_ui
                .search
                .update(cx, |input, cx| input.set_value("", window, cx));
            cx.notify();
        }
    }

    fn settings_save_status(&self) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let (icon, color, message) = if let Some(error) = &self.persistence.error {
            (
                FullIconName::TriangleAlert,
                palette().danger_text,
                format!("保存失败：{error}"),
            )
        } else if disabled {
            (
                FullIconName::Lock,
                palette().muted,
                "当前为只读模式，设置无法修改。".into(),
            )
        } else if self.persistence.busy() {
            (FullIconName::Clock, palette().muted, "正在保存更改…".into())
        } else {
            (
                FullIconName::Check,
                palette().muted,
                "偏好设置自动保存".into(),
            )
        };
        let hint = match self.settings_ui.page {
            3 => "路径选择后立即保存，新会话使用更新后的配置。",
            8 => "服务配置在编辑表单中保存，新会话使用更新后的配置。",
            4 | 9 => "来源文件在编辑器中保存，下次启动会话时加载。",
            10 => "默认值仅用于新会话，已有会话保留原选择。",
            _ => "更改适用于这台设备。",
        };
        h_flex()
            .id("settings-workspace-footer")
            .debug_selector(|| "settings-workspace-footer".into())
            .flex_shrink_0()
            .min_h(px(40.))
            .px_5()
            .py_2()
            .gap_2()
            .flex_wrap()
            .border_t_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface_strong))
            .text_size(px(12.))
            .child(Icon::new(icon).small().text_color(rgb(color)))
            .child(div().text_color(rgb(color)).child(message))
            .child(div().flex_1())
            .child(div().text_color(rgb(palette().muted)).child(hint))
            .into_any_element()
    }
}
