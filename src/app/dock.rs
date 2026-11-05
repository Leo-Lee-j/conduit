//! Shared right-side frame and layout for registered native views.
use super::plugin_ui::right::RightBody;
use super::*;

impl ConduitApp {
    pub(super) fn render_detail_dock(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        self.sync_plugin_context();
        if !self.plugins.right_visible(self.branch_navigation_open()) {
            return (None, None);
        }
        let title = self.plugins.right_title();
        let adapter = self.plugins.right_adapter();
        let view = match (adapter, self.plugins.right_key().cloned()) {
            (Some(adapter), Some(key)) => (adapter.render)(self, &key, window, cx),
            _ => RightBody::Failed("视图暂不可用".into()).into(),
        };
        let choices = self.plugins.right_choices(self.branch_navigation_open());
        let close_id = adapter.map_or("close-right-view", |a| a.close_id);
        let selected = choices.iter().position(|choice| choice.selected);
        let mut tabs = TabBar::new("right-sidebar-tabs")
            .folder()
            .with_size(Size::Medium)
            .h(px(TAB_BAR_HEIGHT))
            .w_full()
            .min_w_0()
            .when_some(selected, |tabs, index| tabs.selected_index(index));
        for choice in choices {
            let selector = format!("right-choice-{}", choice.id);
            let request = choice.request;
            tabs = tabs.child(
                Tab::new()
                    .debug_selector(move || selector.clone())
                    .icon(Icon::new(choice.icon).small())
                    .aria_label(choice.title.clone())
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(choice.title.clone())
                            .build(window, cx)
                    })
                    .disabled(request.is_none())
                    .cursor_pointer()
                    .on_click(cx.listener(move |app, _, window, cx| {
                        if let Some(request) = &request {
                            app.dispatch_right_view(request, window, cx);
                        }
                    })),
            );
        }
        let actions = h_flex()
            .items_center()
            .flex_shrink_0()
            .gap_1()
            .pr_2()
            .children(view.actions)
            .child(
                Button::new(close_id)
                    .debug_selector(move || close_id.into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::X)
                    .tooltip(format!("关闭{title}"))
                    .accessibility_label(format!("关闭{title}"))
                    .on_click(cx.listener(|app, _, window, cx| app.close_right_view(window, cx))),
            );
        let header = h_flex()
            .debug_selector(|| "detail-header".into())
            .h(px(TAB_BAR_HEIGHT))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .child(tabs.suffix(actions));
        let body = match view.body {
            RightBody::Content(body) => body,
            state => {
                let (selector, title, description) = match state {
                    RightBody::Loading => ("right-view-loading", "正在加载…", String::new()),
                    RightBody::Empty { title, description } => {
                        ("detail-empty", title, description.into())
                    }
                    RightBody::Failed(error) => ("right-view-error", "无法显示视图", error),
                    RightBody::Content(_) => unreachable!(),
                };
                v_flex()
                    .debug_selector(move || selector.into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .min_w_0()
                    .p_4()
                    .gap_3()
                    .items_center()
                    .justify_center()
                    .child(div().text_size(px(13.)).font_semibold().child(title))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.6))
                            .text_color(rgb(palette().muted))
                            .child(description),
                    )
                    .into_any_element()
            }
        };
        self.wrap_detail_panel(header.into_any_element(), body, window, cx)
    }

    pub(super) fn wrap_detail_panel(
        &self,
        header: AnyElement,
        body: AnyElement,
        window: &Window,
        cx: &App,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        let narrow = window.viewport_size().width < px(1100.);
        let panel = v_flex()
            .debug_selector(|| "detail-panel".into())
            .occlude()
            .h_full()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .flex_shrink_0()
            // Folder tabs use this same token for their selected shape.
            .bg(cx.theme().tokens.tab_active)
            .text_color(rgb(palette().text))
            .border_l_1()
            .border_color(rgb(palette().border))
            .when(narrow, |panel| {
                panel
                    .w(px(self
                        .plugins
                        .dock
                        .width
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
