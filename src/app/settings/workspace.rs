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
        if (previous_layout.is_some_and(|previous| previous != narrow)
            || (self.settings_ui.page == 1 && self.settings_ui.appearance_modified_only))
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
                h_flex()
                    .id("settings-search-bar")
                    .debug_selector(|| "settings-search-bar".into())
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .border_b_1()
                    .border_color(rgb(palette().border))
                    .child(
                        Button::new("settings-back-to-workbench")
                            .debug_selector(|| "settings-back-to-workbench".into())
                            .small()
                            .ghost()
                            .h(ui_text_size(32.))
                            .icon(FullIconName::ArrowLeft)
                            .label(conduit::i18n::text_static(
                                "settings.workspace.back_to_workbench",
                            ))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_settings(window, cx)),
                            ),
                    )
                    .child(
                        div().flex_1().min_w_0().max_w(px(480.)).child(
                            settings_input(&self.settings_ui.search)
                                .prefix(FullIconName::Search)
                                .aria_label(conduit::i18n::text_static(
                                    "settings.workspace.search_all_settings",
                                ))
                                .cleanable(true),
                        ),
                    ),
            )
            .child(self.settings_body(window, navigation_width, available_width, cx.entity(), cx))
            .child(self.settings_save_status())
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
            .into_any_element()
    }

    pub(super) fn settings_content_pane(
        &self,
        compact: bool,
        searching: bool,
        content: SettingsContent,
        view: Entity<Self>,
    ) -> impl IntoElement {
        let route =
            SettingsRoute::from_index(self.settings_ui.page).unwrap_or(SettingsRoute::General);
        let (section, title, _) = navigation::PAGES[route.navigation_index()];
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .when(compact, |pane| {
                pane.child(
                    div()
                        .id("settings-category-bar")
                        .debug_selector(|| "settings-category-bar".into())
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .flex_shrink_0()
                        .child(self.settings_category_selector(view)),
                )
            })
            .when(!compact, |pane| {
                pane.child(
                    h_flex()
                        .id("settings-page-heading")
                        .debug_selector(|| "settings-page-heading".into())
                        .w_full()
                        .min_w_0()
                        .flex_shrink_0()
                        .px_4()
                        .py_2()
                        .min_h(ui_text_size(36.))
                        .gap_2()
                        .flex_wrap()
                        .border_b_1()
                        .border_color(rgb(palette().border))
                        .text_size(ui_text_size(14.))
                        .when(!searching, |bar| {
                            bar.child(
                                div()
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::source(section)),
                            )
                            .child(
                                Icon::new(FullIconName::ChevronRight)
                                    .small()
                                    .text_color(rgb(palette().muted)),
                            )
                        })
                        .child(div().font_semibold().child(if searching {
                            conduit::i18n::text_static("settings.workspace.search_results")
                        } else {
                            conduit::i18n::source(title)
                        })),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .max_w(px(SETTINGS_CONTENT_MAX_WIDTH))
                    .child(content.into_element()),
            )
    }

    fn settings_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_ui.search.read(cx).value().is_empty() {
            if self.settings_ui.page == 1
                && (!self
                    .settings_ui
                    .appearance_filter
                    .read(cx)
                    .value()
                    .is_empty()
                    || self.settings_ui.appearance_modified_only
                    || self.settings_ui.appearance_group.is_some()
                    || self.settings_ui.group.is_some())
            {
                self.clear_appearance_filters(window, cx);
                return;
            }
            if self.settings_ui.page == 3 && self.connection_settings.engine_detail_open {
                self.connection_settings.engine_detail_open = false;
                self.settings_ui.focus.focus(window, cx);
                cx.notify();
            } else if self.settings_ui.page == 8 && self.connection_settings.service_detail_open {
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

    fn settings_save_status(&self) -> AnyElement {
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
            .into_any_element()
    }
}
