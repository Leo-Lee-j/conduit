//! A retained settings destination. Forms may open dialogs above it.
use super::*;

impl ConduitApp {
    pub(in crate::app) fn render_settings_workspace(
        &self,
        window: &mut Window,
        available_width: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let navigation_width =
            navigation::settings_navigation_width(available_width, self.store.ui.ui_font_size);
        let narrow = navigation_width == 0.;
        let previous_layout = self.settings_ui.layout_compact.replace(Some(narrow));
        if previous_layout.is_some_and(|previous| previous != narrow)
            && window.has_active_dialog(cx)
        {
            self.settings_ui.restore_navigation_focus.set(true);
        }
        if self.settings_ui.restore_navigation_focus.get()
            && !window.has_active_dialog(cx)
            && window.focused(cx).is_some()
        {
            self.settings_ui.restore_navigation_focus.set(false);
        }
        if self.settings_ui.restore_navigation_focus.get()
            && self.settings_ui.open
            && !window.has_active_dialog(cx)
            && window.focused(cx).is_none()
        {
            self.settings_ui.restore_navigation_focus.set(false);
            let view = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    if this.settings_ui.open
                        && !window.has_active_dialog(cx)
                        && window.focused(cx).is_none()
                    {
                        this.settings_ui
                            .search
                            .read(cx)
                            .focus_handle(cx)
                            .focus(window, cx);
                    }
                });
            });
        }
        if previous_layout.is_some_and(|previous| previous != narrow)
            && !window.has_active_dialog(cx)
            && self.settings_ui.focus.contains_focused(window, cx)
            && let Some(previous_focus) = window.focused(cx)
        {
            // A navigation button disappears when the rail becomes a category
            // control (and vice versa). Repair only that orphaned focus after
            // layout. Retained text editors and focus outside settings are untouched.
            let view = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| {
                let _ = view.update(cx, |this, cx| {
                    if this.settings_ui.open
                        && !window.has_active_dialog(cx)
                        && previous_focus.is_focused(window)
                        && !this.settings_ui.focus.contains_focused(window, cx)
                    {
                        this.settings_ui
                            .search
                            .read(cx)
                            .focus_handle(cx)
                            .focus(window, cx);
                    }
                });
            });
        }
        let compact = narrow || f32::from(window.viewport_size().height) < 650.;
        let searching = !self.settings_ui.search.read(cx).value().trim().is_empty();
        let page = navigation::SettingsRoute::from_index(self.settings_ui.page)
            .unwrap_or(navigation::SettingsRoute::General)
            .index();
        let title = if searching {
            conduit::i18n::text_static("settings.summary.search_title")
        } else {
            conduit::i18n::source(navigation::PAGES[page].1)
        };
        let summary = conduit::i18n::text_static(if searching {
            "settings.summary.search"
        } else {
            navigation::PAGE_SUMMARIES[page]
        });
        let back = Button::new("settings-back")
            .debug_selector(|| "settings-close".into())
            .icon(FullIconName::ArrowLeft)
            .label(conduit::i18n::text_static("files.settings"))
            .accessibility_label(conduit::i18n::text_static(
                "settings.workspace.back_to_workbench",
            ))
            .ghost()
            .tooltip(conduit::i18n::format(
                "settings.workspace.back_to_workbench_a9f951",
                &[(
                    "arg0",
                    shortcuts::display("settings_back", &self.store.ui.shortcuts).to_string(),
                )],
            ))
            .on_click(cx.listener(|this, _, window, cx| this.close_settings(window, cx)));
        let heading = v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                div()
                    .id("settings-page-title")
                    .debug_selector(|| "settings-page-title".into())
                    .text_size(ui_text_size(if compact { 18. } else { 20. }))
                    .line_height(ui_text_size(if compact { 24. } else { 28. }))
                    .font_semibold()
                    .child(title),
            )
            .when(!compact, |heading| {
                heading.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .line_height(ui_text_size(16.))
                        .text_color(rgb(palette().muted))
                        .child(summary),
                )
            });
        let search = Input::new(&self.settings_ui.search)
            .prefix(FullIconName::Search)
            .aria_label(conduit::i18n::text_static(
                "settings.workspace.search_all_settings",
            ))
            .cleanable(true);
        let header = if narrow {
            v_flex()
                .w_full()
                .min_w_0()
                .px_4()
                .py_3()
                .gap_3()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_3()
                        .child(back)
                        .child(heading),
                )
                .child(search)
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .min_w_0()
                .child(
                    h_flex()
                        .w(px(navigation_width))
                        .flex_shrink_0()
                        .px_4()
                        .py_3()
                        .child(back),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .px_4()
                        .py_3()
                        .gap_4()
                        .child(heading)
                        .child(
                            div()
                                .w(px(260.))
                                .max_w(relative(0.45))
                                .flex_shrink_0()
                                .child(search),
                        ),
                )
                .into_any_element()
        };
        v_flex()
            .id("settings-workspace")
            .key_context("ConduitSettings")
            .test_support()
            .debug_selector(|| "settings-workspace".into())
            .track_focus(&self.settings_ui.focus)
            .flex_1()
            .min_h_0()
            .w_full()
            .min_w_0()
            .bg(chrome_background(palette().canvas))
            .text_color(rgb(palette().text))
            .child(
                div()
                    .id("settings-workspace-header")
                    .debug_selector(|| "settings-workspace-header".into())
                    .w_full()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(header),
            )
            .when(narrow, |workspace| {
                workspace.child(
                    h_flex()
                        .id("settings-category-bar")
                        .debug_selector(|| "settings-category-bar".into())
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .gap_3()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static("settings.workspace.category")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(self.settings_category_selector(cx.entity())),
                        ),
                )
            })
            .child(
                div()
                    .id("settings-workspace-body")
                    .debug_selector(|| "settings-workspace-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .min_w_0()
                    .child(self.settings_body(window, available_width, cx.entity(), cx)),
            )
            .child(self.settings_save_status(compact))
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
            if self.settings_ui.page == 8 && self.connection_settings.service_detail_open {
                self.connection_settings.service_detail_open = false;
                self.settings_ui.focus.focus(window, cx);
                cx.notify();
            } else {
                self.close_settings(window, cx);
            }
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
            8 => conduit::i18n::text_static("settings.workspace.services_scope"),
            4 | 9 => {
                conduit::i18n::text_static("settings.workspace.source_files_save_in_the_editor_and")
            }
            5 => conduit::i18n::text_static("settings.workspace.naming_scope"),
            10 => conduit::i18n::text_static("settings.workspace.session_scopes"),
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
            .bg(chrome_background(palette().canvas))
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
