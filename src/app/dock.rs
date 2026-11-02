//! Tool details dock in wide and narrow windows; Git lives in left navigation.

use super::*;

fn detail_section(
    scope: &str,
    name: &str,
    label: &str,
    text: &str,
    empty: &str,
    typography: &ChatTypography,
) -> AnyElement {
    let text = format_tool_text(text);
    let copy = text.clone();
    v_flex()
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .overflow_hidden()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .child(
            h_flex()
                .h(px(36.))
                .px_3()
                .items_center()
                .border_b_1()
                .border_color(rgb(palette().border))
                .bg(rgb(palette().surface_strong))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_semibold()
                        .child(label.to_owned()),
                )
                .child(
                    Button::new(SharedString::from(format!("{scope}-{name}-copy")))
                        .ghost()
                        .small()
                        .compact()
                        .icon(FullIconName::Copy)
                        .tooltip(format!("复制{label}"))
                        .accessibility_label(format!("复制{label}"))
                        .disabled(text.trim().is_empty())
                        .cursor_pointer()
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                        }),
                ),
        )
        .child(
            div()
                .debug_selector({
                    let name = name.to_owned();
                    move || format!("detail-{name}")
                })
                .w_full()
                .min_w_0()
                .p_3()
                .font_family(typography.code_font.clone())
                .text_size(px(typography.code_size))
                .line_height(relative(1.6))
                .text_color(rgb(if text.trim().is_empty() {
                    palette().muted
                } else {
                    palette().text
                }))
                .child(gpui_kit::base::SelectableText::new(
                    SharedString::from(format!("{scope}-{name}-text")),
                    if text.trim().is_empty() {
                        empty.to_owned()
                    } else {
                        text
                    },
                )),
        )
        .into_any_element()
}

impl ConduitApp {
    pub(super) fn render_detail_dock(
        &mut self,
        selected: Option<&SessionSummary>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        if !self.details {
            return (None, None);
        }
        let narrow = window.viewport_size().width < px(1100.);
        let item = self.detail_text.as_ref().and_then(|key| {
            self.session(self.selected_session?)?
                .messages
                .iter()
                .find(|m| &m.key == key)
                .cloned()
        });
        let typography = self.chat_typography();
        let mut header = h_flex()
            .debug_selector(|| "detail-header".into())
            .h(px(48.))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .px_3()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(
                Icon::new(FullIconName::PanelRight)
                    .small()
                    .text_color(rgb(palette().muted)),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_semibold()
                    .child("工具详情"),
            );
        if let Some(item) = &item {
            let copy = format!(
                "输入\n{}\n\n输出 / 结果\n{}",
                format_tool_text(&item.input),
                format_tool_text(&item.detail)
            );
            header = header.child(
                Button::new("copy-details")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Copy)
                    .tooltip("复制全部输入与输出")
                    .accessibility_label("复制全部输入与输出")
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
            );
        }
        header = header.child(
            Button::new("close-details")
                .ghost()
                .small()
                .compact()
                .icon(FullIconName::X)
                .tooltip("关闭侧边栏")
                .accessibility_label("关闭工具详情")
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.details = false;
                    cx.notify();
                })),
        );
        let body: AnyElement = if let Some(item) = item {
            let scope = format!(
                "dock-{}-{}",
                self.selected_session.unwrap_or_default(),
                item.key
            );
            let scroll = self
                .detail_scrolls
                .entry((self.selected_session.unwrap_or_default(), item.key.clone()))
                .or_default()
                .clone();
            let running = tool_running(&item);
            let failed = tool_failed(&item);
            let status = if running {
                "执行中"
            } else if failed {
                "未成功"
            } else if item.status.is_empty() {
                "未提供状态"
            } else {
                &item.status
            };
            let color = if running {
                palette().activity_running
            } else if failed {
                palette().diff_remove_text
            } else {
                palette().diff_add_text
            };
            let path = self
                .selected_workspace
                .map(|i| display_path(&self.store.workspaces[i].path))
                .unwrap_or_default();
            let mut content = v_flex()
                .w_full()
                .min_w_0()
                .flex_shrink_0()
                .p_3()
                .gap_3()
                .child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .font_semibold()
                                        .child(tool_category(&item)),
                                )
                                .child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .items_center()
                                        .gap_1()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(6.))
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(px(11.))
                                        .text_color(rgb(color))
                                        .child(
                                            Icon::new(if running {
                                                FullIconName::Clock
                                            } else if failed {
                                                FullIconName::CircleX
                                            } else {
                                                FullIconName::CircleCheck
                                            })
                                            .small(),
                                        )
                                        .child(status.to_owned()),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette().muted))
                                .child(item.text.clone()),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child(path),
                        ),
                )
                .child(detail_section(
                    &scope,
                    "input",
                    "输入",
                    &item.input,
                    "未提供输入",
                    &typography,
                ))
                .child(detail_section(
                    &scope,
                    "output",
                    "输出 / 结果",
                    &item.detail,
                    if running {
                        "等待工具输出…"
                    } else {
                        "没有输出内容"
                    },
                    &typography,
                ));
            if let Some(native_id) = selected.and_then(|s| s.native_id.clone()) {
                content = content.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette().muted))
                                .child("原生会话 ID"),
                        )
                        .child(gpui_kit::base::SelectableText::new(
                            SharedString::from(format!("{scope}-native-id")),
                            native_id,
                        )),
                );
            }
            div()
                .id(SharedString::from(scope))
                .debug_selector(|| "detail-content".into())
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&scroll)
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(content)
                .vertical_scrollbar(&scroll)
                .into_any_element()
        } else {
            v_flex()
                .debug_selector(|| "detail-empty".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .min_w_0()
                .p_4()
                .gap_3()
                .items_center()
                .justify_center()
                .child(Icon::new(FullIconName::PanelRight).text_color(rgb(palette().muted)))
                .child(
                    div()
                        .text_size(px(13.))
                        .font_semibold()
                        .child("选择一个工具调用"),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(relative(1.6))
                        .text_color(rgb(palette().muted))
                        .child("展开聊天中的工具卡片，点击详情图标，在这里查看完整输入与输出。"),
                )
                .into_any_element()
        };
        let panel = v_flex()
            .debug_selector(|| "detail-panel".into())
            .occlude()
            .h_full()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .flex_shrink_0()
            .bg(rgb(palette().sidebar_bg))
            .text_color(rgb(palette().text))
            .border_l_1()
            .border_color(rgb(palette().border))
            .when(narrow, |panel| {
                panel
                    .w(px(self
                        .detail_width
                        .min(f32::from(window.viewport_size().width) - 38.)))
                    .absolute()
                    .top_0()
                    .right_0()
                    .h_auto()
                    .bottom(px(34.))
                    .shadow_lg()
            })
            .child(header)
            .child(body)
            .into_any_element();
        if narrow {
            (None, Some(panel))
        } else {
            (Some(panel), None)
        }
    }
}
