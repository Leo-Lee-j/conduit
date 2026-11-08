//! A retained settings destination. Forms may open dialogs above it.
use super::*;

impl ConduitApp {
    pub(in crate::app) fn render_settings_workspace(
        &self,
        window: &Window,
        available_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let narrow = available_width < 720.;
        v_flex()
            .id("settings-workspace")
            .key_context("ConduitSettings")
            .test_support()
            .debug_selector(|| "settings-workspace".into())
            .track_focus(&self.settings_ui.focus)
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(
                h_flex()
                    .id("settings-workspace-header")
                    .debug_selector(|| "settings-workspace-header".into())
                    .h(px(52.))
                    .flex_shrink_0()
                    .px_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new("settings-back")
                            .debug_selector(|| "settings-close".into())
                            .icon(FullIconName::ArrowLeft)
                            .accessibility_label(conduit::i18n::text_static(
                                "settings.workspace.back_to_workbench",
                            ))
                            .when(!narrow, |button| {
                                button.label(conduit::i18n::text_static(
                                    "settings.workspace.back_to_workbench",
                                ))
                            })
                            .ghost()
                            .tooltip(conduit::i18n::format(
                                "settings.workspace.back_to_workbench_a9f951",
                                &[(
                                    "arg0",
                                    shortcuts::display("settings_back", &self.store.ui.shortcuts)
                                        .to_string(),
                                )],
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                            ),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.settings_ui.search)
                                .prefix(FullIconName::Search)
                                .aria_label(conduit::i18n::text_static(
                                    "settings.workspace.search_all_settings",
                                ))
                                .cleanable(true),
                        ),
                    ),
            )
            .child(
                div()
                    .id("settings-workspace-body")
                    .debug_selector(|| "settings-workspace-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.settings_body(window, available_width, cx.entity(), cx)),
            )
            .child(self.settings_save_status(narrow))
            .on_action(cx.listener(|this, _: &OpenSettingsShortcut, window, cx| {
                this.settings_ui
                    .search
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusSettingsSearch, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.settings_ui
                        .search
                        .read(cx)
                        .focus_handle(cx)
                        .focus(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &LeaveSettings, window, cx| {
                if !window.has_active_dialog(cx) {
                    this.settings_escape(window, cx);
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                if event.keystroke.key == "enter"
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

    fn settings_save_status(&self, narrow: bool) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let (icon, color, message) = if let Some(error) = &self.persistence.error {
            (
                FullIconName::TriangleAlert,
                palette().danger_text,
                conduit::i18n::format("notifications.save_failed", &[("error", error.to_string())]),
            )
        } else if disabled {
            (
                FullIconName::Lock,
                palette().muted,
                conduit::i18n::text_static(
                    "settings.workspace.read_only_mode_settings_cannot_be_changed",
                )
                .into(),
            )
        } else if self.persistence.busy() {
            (
                FullIconName::Clock,
                palette().muted,
                conduit::i18n::text_static("settings.workspace.saving_changes").into(),
            )
        } else {
            (
                FullIconName::Check,
                palette().muted,
                conduit::i18n::text_static("settings.workspace.preferences_save_automatically")
                    .into(),
            )
        };
        let hint = match self.settings_ui.page {
            3 => conduit::i18n::text_static(
                "settings.workspace.paths_save_immediately_new_sessions_use_the",
            ),
            8 => conduit::i18n::text_static(
                "settings.workspace.service_configuration_saves_in_the_edit_form",
            ),
            4 | 9 => {
                conduit::i18n::text_static("settings.workspace.source_files_save_in_the_editor_and")
            }
            10 => conduit::i18n::text_static(
                "settings.workspace.defaults_apply_only_to_new_sessions_existing",
            ),
            _ => conduit::i18n::text_static("settings.workspace.changes_apply_to_this_device"),
        };
        h_flex()
            .id("settings-workspace-footer")
            .debug_selector(|| "settings-workspace-footer".into())
            .flex_shrink_0()
            .min_h(px(32.))
            .px_4()
            .py_1()
            .gap_2()
            .flex_wrap()
            .border_t_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().canvas))
            .text_size(ui_text_size(12.))
            .child(Icon::new(icon).small().text_color(rgb(color)))
            .child(div().text_color(rgb(color)).child(message))
            .when(!narrow, |footer| {
                footer
                    .child(div().flex_1())
                    .child(div().text_color(rgb(palette().muted)).child(hint))
            })
            .into_any_element()
    }
}
