//! Tool details dock in wide and narrow windows; Git lives in left navigation.

use super::*;

impl ConduitApp {
    pub(super) fn render_detail_dock(
        &self,
        selected: Option<&SessionSummary>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let narrow = window.viewport_size().width < px(1100.);
        let mut detail_panel: Option<AnyElement> = None;
        let mut overlay: Option<AnyElement> = None;
        if self.details {
            let native_id = selected
                .as_ref()
                .and_then(|s| s.native_id.clone())
                .unwrap_or_else(|| "首次回复后绑定".into());
            let path = self
                .selected_workspace
                .map(|i| display_path(&self.store.workspaces[i].path))
                .unwrap_or_default();
            let tool_content = self
                .detail_text
                .as_ref()
                .and_then(|key| {
                    self.session(self.selected_session?)?
                        .messages
                        .iter()
                        .find(|m| &m.key == key)
                })
                .map(|item| {
                    format!(
                        "输入\n{}\n\n输出 / 结果\n{}",
                        if item.input.is_empty() {
                            "未提供"
                        } else {
                            &item.input
                        },
                        if item.detail.is_empty() {
                            "尚未返回"
                        } else {
                            &item.detail
                        }
                    )
                })
                .unwrap_or_else(|| {
                    "展开工具调用卡片后，点击右侧详情图标可在此查看完整输入与输出。".into()
                });
            let tool_body: AnyElement = v_flex()
                .flex_1()
                .min_h_0()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("工作目录"),
                )
                .child(div().text_xs().text_color(rgb(palette().text)).child(path))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child("原生会话 ID"),
                )
                .child(div().text_xs().child(native_id))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .id("detail-content")
                        .overflow_y_scroll()
                        .text_xs()
                        .child(tool_content),
                )
                .into_any_element();
            let body = tool_body;
            let panel = v_flex()
                .h_full()
                .w_full()
                .flex_shrink_0()
                .bg(rgb(palette().sidebar_bg))
                .p_3()
                .gap_3()
                .when(narrow, |panel| {
                    panel
                        .w(px(300.))
                        .absolute()
                        .top_0()
                        .right_0()
                        .h_auto()
                        .bottom(px(34.))
                        .border_l_1()
                        .border_color(rgb(palette().border))
                })
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Icon::new(gpui_kit::assets::IconName::PanelRight)
                                .small()
                                .text_color(rgb(palette().muted)),
                        )
                        .child(div().text_size(px(12.)).font_semibold().child("工具详情"))
                        .child(div().flex_1())
                        .child(
                            Button::new("close-details")
                                .ghost()
                                .small()
                                .compact()
                                .icon(gpui_kit::assets::IconName::X)
                                .cursor_pointer()
                                .tooltip("关闭侧边栏")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.details = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(body);
            if narrow {
                overlay = Some(panel.into_any_element());
            } else {
                detail_panel = Some(panel.into_any_element());
            }
        }
        (detail_panel, overlay)
    }
}
