//! Shared layout for CLI, model, Skill and MCP management.
use super::*;

pub(super) fn management_badge(label: impl Into<SharedString>, active: bool) -> Div {
    div()
        .flex_shrink_0()
        .px_2()
        .py(px(2.))
        .rounded_md()
        .bg(if active {
            gpui_kit::rgba((palette().activity_running << 8) | 0x18)
        } else {
            rgb(palette().surface_strong)
        })
        .text_size(ui_text_size(11.))
        .text_color(rgb(if active {
            palette().activity_running
        } else {
            palette().muted
        }))
        .child(label.into())
}

pub(super) fn engine_tabs(
    engine: Provider,
    extensions: bool,
    view: Entity<ConduitApp>,
) -> AnyElement {
    let mut tabs = h_flex()
        .id("management-engines")
        .debug_selector(|| "management-engines".into())
        .w_full()
        .min_w_0()
        .flex_wrap()
        .gap_1()
        .p_1()
        .rounded_lg()
        .border_1()
        .border_color(settings_group_border());
    for (index, candidate) in Provider::ALL.into_iter().enumerate() {
        let view = view.clone();
        let selected = candidate == engine;
        tabs = tabs.child(
            Button::new(("management-engine", index))
                .debug_selector(move || {
                    format!(
                        "{}-engine-{index}",
                        if extensions { "extensions" } else { "settings" }
                    )
                })
                .accessibility_label(candidate.label())
                .tooltip(candidate.label())
                .ghost()
                .selected(selected)
                .flex_shrink_0()
                .px_3()
                .h(px(32.))
                .rounded_md()
                .child(
                    h_flex()
                        .debug_selector(move || format!("management-engine-content-{index}"))
                        .flex_shrink_0()
                        .gap_2()
                        .child(provider_icon(candidate))
                        .child(div().whitespace_nowrap().child(candidate.label())),
                )
                .when(selected, |tab| {
                    tab.bg(rgb(palette().surface))
                        .text_color(rgb(palette().text))
                        .font_semibold()
                })
                .on_click(move |_, window, cx| {
                    view.update(cx, |app, cx| {
                        if extensions {
                            app.extensions_ui.engine = candidate;
                            app.extensions_ui.feedback = None;
                            app.ensure_extensions(window, cx, false);
                        } else {
                            app.connection_settings.engine = candidate;
                            app.connection_settings.service_selection = None;
                            app.connection_settings.service_detail_open = false;
                            app.connection_settings.service_tab =
                                super::services::DetailTab::Models;
                            app.connection_settings.models_expanded = false;
                            app.connection_settings.sources_expanded = false;
                            app.connection_settings.model_limit = 50;
                            app.connection_settings
                                .model_search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            app.settings_ui.group = None;
                        }
                        cx.notify();
                    })
                }),
        );
    }
    tabs.into_any_element()
}

/// Engine choices use the full pane width and wrap only when their labels
/// no longer fit. Page actions stay on a separate row.
pub(super) fn management_toolbar(
    engine: Provider,
    extensions: bool,
    mode_tabs: Option<AnyElement>,
    actions: Option<AnyElement>,
    view: Entity<ConduitApp>,
) -> AnyElement {
    let inset = settings_content_inset();
    let tabs = engine_tabs(engine, extensions, view);
    h_flex()
        .debug_selector(|| "management-engine-bar".into())
        .w_full()
        .min_w_0()
        .flex_shrink_0()
        .flex_wrap()
        .gap_2()
        .px(px(inset))
        .pt(px(inset))
        .child(tabs)
        .when_some(mode_tabs, |bar, modes| bar.child(modes))
        .when_some(actions, |bar, actions| bar.child(actions))
        .into_any_element()
}
