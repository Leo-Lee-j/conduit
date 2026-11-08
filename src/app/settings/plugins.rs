//! Plugin management is separate from CLI Skills and MCP services.
use super::*;
mod catalog;
fn text(key: &str) -> String {
    conduit::i18n::text(&format!("settings.plugins.{key}"))
}
fn caption(value: impl Into<SharedString>) -> Div {
    div()
        .text_size(ui_text_size(12.))
        .line_height(relative(1.5))
        .text_color(rgb(palette().muted))
        .child(value.into())
}
impl ConduitApp {
    fn settings_installed_plugins_content(&self, view: Entity<Self>) -> AnyElement {
        let state = &self.plugins.packages;
        let disabled = !self.writable || self.closing || state.busy;
        let install_view = view.clone();
        let refresh_view = view.clone();
        let default_view = view.clone();
        let default_active = self.store.ui.plugins.file_icon_theme.is_none();
        let mut content = v_flex()
            .id("settings-plugins-list")
            .debug_selector(|| "settings-plugins-list".into())
            .flex_1()
            .min_h_0()
            .h_full()
            .w_full()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("plugin-install")
                            .debug_selector(|| "plugin-install".into())
                            .icon(FullIconName::Plus)
                            .label(text("install"))
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                install_view
                                    .update(cx, |app, cx| app.install_plugin_folder(window, cx))
                            }),
                    )
                    .child(
                        Button::new("plugin-refresh")
                            .debug_selector(|| "plugin-refresh".into())
                            .icon(IconName::RefreshCw)
                            .label(text("refresh"))
                            .disabled(state.busy || self.closing)
                            .on_click(move |_, _, cx| {
                                refresh_view.update(cx, |app, cx| app.refresh_installed_plugins(cx))
                            }),
                    )
                    .child(
                        Button::new("plugin-folder")
                            .label(text("open_folder"))
                            .ghost()
                            .disabled(disabled)
                            .on_click(|_, _, cx| {
                                let root = crate::app::plugin_ui::management::installation_root();
                                if std::fs::create_dir_all(&root).is_ok() {
                                    cx.reveal_path(&root);
                                }
                            }),
                    ),
            )
            .child(caption(text("hint")))
            .when_some(state.feedback.clone(), |list, message| {
                list.child(caption(message))
            })
            .child(
                v_flex()
                    .p_4()
                    .gap_2()
                    .rounded_md()
                    .bg(rgb(if is_dark_mode() {
                        palette().surface_strong
                    } else {
                        palette().surface_hover
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Icon::new(FullIconName::Blocks).small())
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_medium()
                                    .child(text("core")),
                            )
                            .child(caption(text("built_in"))),
                    )
                    .child(caption(text("core_description"))),
            )
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .rounded_md()
                    .bg(rgb(if is_dark_mode() {
                        palette().surface_strong
                    } else {
                        palette().surface_hover
                    }))
                    .child(
                        div()
                            .text_size(ui_text_size(14.))
                            .font_medium()
                            .child(text("default_icons")),
                    )
                    .child(caption(text("default_description")))
                    .child(
                        Button::new("plugin-default-icons")
                            .debug_selector(|| "plugin-default-icons".into())
                            .label(text(if default_active {
                                "active"
                            } else {
                                "use_icons"
                            }))
                            .selected(default_active)
                            .disabled(disabled || default_active)
                            .on_click(move |_, _, cx| {
                                default_view
                                    .update(cx, |app, cx| app.select_file_icon_theme(None, cx))
                            }),
                    ),
            );
        if state.busy && !state.initialized {
            content = content.child(caption(text("loading")));
        }
        for entry in &state.entries {
            let manifest = &entry.package.manifest;
            let enabled = entry.registration.is_some();
            let active =
                self.store.ui.plugins.file_icon_theme.as_deref() == Some(&manifest.id) && enabled;
            let id = manifest.id.clone();
            let toggle_id = id.clone();
            let select_id = id.clone();
            let uninstall_id = id.clone();
            let toggle_view = view.clone();
            let select_view = view.clone();
            let uninstall_view = view.clone();
            let name = manifest.name.clone();
            content = content.child(
                v_flex()
                    .id(SharedString::from(format!("plugin-{id}")))
                    .debug_selector(move || format!("plugin-{id}"))
                    .p_4()
                    .gap_3()
                    .rounded_md()
                    .bg(rgb(if is_dark_mode() {
                        palette().surface_strong
                    } else {
                        palette().surface_hover
                    }))
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_medium()
                                    .child(manifest.name.clone()),
                            )
                            .child(caption(manifest.version.clone()))
                            .child(caption(text(if enabled { "enabled" } else { "disabled" }))),
                    )
                    .child(caption(manifest.description.clone()))
                    .child(caption(format!("{} · {}", text("file_icons"), manifest.id)))
                    .when_some(entry.package.source.as_ref(), |card, source| {
                        card.child(caption(format!("GitHub · {}", source.repository())))
                    })
                    .child(
                        h_flex().gap_3().children(
                            [
                                Path::new("src"),
                                Path::new("main.rs"),
                                Path::new("settings.json"),
                                Path::new("README.md"),
                            ]
                            .into_iter()
                            .map(|path| {
                                // Preview this package without changing the user's active theme.
                                let icon = manifest
                                    .file_icons
                                    .resolve(path, path == Path::new("src"), false)
                                    .and_then(|key| entry.package.assets.get(key))
                                    .map(|bytes| Icon::default().data(bytes))
                                    .unwrap_or_else(|| Icon::new(IconName::File));
                                icon.size(px(20.))
                                    .text_color(rgb(palette().muted))
                                    .into_any_element()
                            }),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("plugin-use-icons")
                                    .debug_selector({
                                        let id = select_id.clone();
                                        move || format!("plugin-use-{id}")
                                    })
                                    .label(text(if active { "active" } else { "use_icons" }))
                                    .selected(active)
                                    .disabled(disabled || !enabled || active)
                                    .on_click(move |_, _, cx| {
                                        select_view.update(cx, |app, cx| {
                                            app.select_file_icon_theme(Some(select_id.clone()), cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new("plugin-toggle")
                                    .debug_selector({
                                        let id = toggle_id.clone();
                                        move || format!("plugin-toggle-{id}")
                                    })
                                    .label(text(if enabled { "disable" } else { "enable" }))
                                    .disabled(disabled)
                                    .on_click(move |_, _, cx| {
                                        toggle_view.update(cx, |app, cx| {
                                            app.set_plugin_enabled(&toggle_id, !enabled, cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new("plugin-uninstall")
                                    .debug_selector({
                                        let id = uninstall_id.clone();
                                        move || format!("plugin-uninstall-{id}")
                                    })
                                    .label(text("uninstall"))
                                    .ghost()
                                    .disabled(disabled)
                                    .on_click(move |_, window, cx| {
                                        let view = uninstall_view.clone();
                                        let id = uninstall_id.clone();
                                        let name = name.clone();
                                        window.open_dialog(cx, move |dialog, _, _| {
                                            let view = view.clone();
                                            let id = id.clone();
                                            dialog
                                                .title(text("confirm_title"))
                                                .child(caption(conduit::i18n::format(
                                                    "settings.plugins.confirm_body",
                                                    &[("name", name.clone())],
                                                )))
                                                .footer(
                                                    h_flex()
                                                        .justify_end()
                                                        .gap_2()
                                                        .child(
                                                            Button::new("plugin-uninstall-cancel")
                                                                .label(text("cancel"))
                                                                .on_click(|_, window, cx| {
                                                                    window.close_dialog(cx)
                                                                }),
                                                        )
                                                        .child(
                                                            Button::new("plugin-uninstall-confirm")
                                                                .debug_selector(|| {
                                                                    "plugin-uninstall-confirm"
                                                                        .into()
                                                                })
                                                                .label(text("uninstall"))
                                                                .on_click(move |_, window, cx| {
                                                                    view.update(cx, |app, cx| {
                                                                        app.uninstall_plugin(
                                                                            id.clone(),
                                                                            cx,
                                                                        )
                                                                    });
                                                                    window.close_dialog(cx);
                                                                }),
                                                        ),
                                                )
                                        });
                                    }),
                            ),
                    ),
            );
        }
        if state.entries.is_empty() && !state.busy {
            content = content.child(
                v_flex()
                    .py_4()
                    .gap_2()
                    .child(div().text_size(ui_text_size(14.)).child(text("empty")))
                    .child(caption(text("empty_description"))),
            );
        }
        if !state.errors.is_empty() {
            content = content.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_color(rgb(palette().error_text))
                            .child(text("errors")),
                    )
                    .children(state.errors.iter().map(|error| caption(error.clone()))),
            );
        }
        content.into_any_element()
    }
}
