//! Shared right-side frame and layout for registered native views.
use super::plugin_ui::right::RightBody;
use super::*;

// The tab strip depends on selection/eligibility, never on the draft text.
// Keep it separate from the body: refreshing an ancestor cache would discard
// the independently cached ScriptView when a command changes its contents.
#[derive(PartialEq, Eq)]
struct HeaderState {
    branches: bool,
    close_id: &'static str,
    appearance: (u32, u32, u32),
}

struct DockHeader {
    app: WeakEntity<ConduitApp>,
    state: Option<HeaderState>,
    revision: Option<super::plugin_ui::right::RightHeaderRevision>,
}

impl Render for DockHeader {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let close_id = self
            .state
            .as_ref()
            .expect("header initialized before rendering")
            .close_id;
        self.app
            .update(cx, |app, cx| {
                app.render_detail_header(Vec::new(), close_id, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl ConduitApp {
    pub(super) fn render_detail_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<AnyElement>) {
        self.sync_plugin_context();
        if !self.plugins.right_visible(self.branch_navigation_open()) {
            return (None, None);
        }
        let adapter = self.plugins.right_adapter();
        let view = match (adapter, self.plugins.right_key().cloned()) {
            (Some(adapter), Some(key)) => (adapter.render)(self, &key, window, cx),
            _ => RightBody::Failed("视图暂不可用".into()).into(),
        };
        let close_id = adapter.map_or("close-right-view", |a| a.close_id);
        let header = if view.actions.is_empty() {
            let app = cx.weak_entity();
            let header =
                window.use_keyed_state("detail-header-cache", cx, move |_, _| DockHeader {
                    app,
                    state: None,
                    revision: None,
                });
            let state = HeaderState {
                branches: self.branch_navigation_open(),
                close_id,
                appearance: (palette().canvas, palette().border, palette().text),
            };
            header.update(cx, |header, cx| {
                if header.state.as_ref() != Some(&state)
                    || !self.plugins.right_header_matches(header.revision.as_ref())
                {
                    header.state = Some(state);
                    header.revision = Some(self.plugins.right_header_revision());
                    cx.notify();
                }
            });
            header
                .cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(px(TAB_BAR_HEIGHT))
                        .flex_shrink_0(),
                )
                .into_any_element()
        } else {
            self.render_detail_header(view.actions, close_id, cx)
        };
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let header = chat::profiling::host_element(
            header,
            [
                "dock_header_layout",
                "dock_header_prepaint",
                "dock_header_paint",
            ],
        );
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
                    .id(selector)
                    .test_support()
                    .debug_selector(move || selector.into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .min_w_0()
                    .p_4()
                    .gap_3()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_semibold()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .line_height(relative(1.6))
                            .text_color(rgb(palette().muted))
                            .child(description),
                    )
                    .into_any_element()
            }
        };
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let body = chat::profiling::host_element(
            body,
            ["dock_body_layout", "dock_body_prepaint", "dock_body_paint"],
        );
        self.wrap_detail_panel(header, body, window, cx)
    }

    fn render_detail_header(
        &self,
        actions: Vec<AnyElement>,
        close_id: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let _span = chat::profiling::host_span("dock_header_build");
        let title = self.plugins.right_title();
        let choices = self.plugins.right_choices(self.branch_navigation_open());
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
            .children(actions)
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
            .id("detail-header")
            .test_support()
            .debug_selector(|| "detail-header".into())
            .h(px(TAB_BAR_HEIGHT))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .child(tabs.suffix(actions));
        header.into_any_element()
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
            .id("detail-panel")
            .test_support()
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
