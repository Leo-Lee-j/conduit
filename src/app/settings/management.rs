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

fn management_actions(actions: AnyElement, narrow: bool) -> Div {
    h_flex()
        .debug_selector(|| "settings-page-actions".into())
        .w_full()
        .flex_shrink_0()
        .justify_end()
        .px(px(if narrow { 12. } else { 24. }))
        .py(px(8.))
        .child(actions)
}

fn engine_tabs(
    engine: Provider,
    extensions: bool,
    columns: u16,
    view: Entity<ConduitApp>,
) -> AnyElement {
    let mut tabs = div()
        .grid()
        .grid_cols(columns)
        .id("management-engines")
        .debug_selector(|| "management-engines".into())
        .w_full()
        .max_w(rems(36.))
        .gap_1()
        .p_1()
        .rounded_lg()
        .bg(rgb(palette().surface_strong));
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
                .w_full()
                .min_w_0()
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

/// Give engine choices and page actions separate rows so the full set of
/// labels can wrap without clipping either controls or content.
pub(super) fn management_toolbar(
    engine: Provider,
    extensions: bool,
    actions: Option<AnyElement>,
    available_width: f32,
    font_size: f32,
    view: Entity<ConduitApp>,
) -> AnyElement {
    // Budget for complete labels, icons, button padding and grid gaps. Change
    // the column count with the pane width and UI font, never clip a label.
    let inner_width = (available_width - 24.).max(0.);
    let scale = font_size / DEFAULT_UI_FONT_SIZE;
    let columns = if inner_width >= 548. * scale {
        4
    } else if inner_width >= 272. * scale {
        2
    } else {
        1
    };
    let narrow = available_width < 540.;
    let tabs = engine_tabs(engine, extensions, columns, view);
    v_flex()
        .w_full()
        .flex_shrink_0()
        .child(div().w_full().px_3().py_2().child(tabs))
        .when_some(actions, |bar, actions| {
            bar.child(management_actions(actions, narrow))
        })
        .into_any_element()
}
