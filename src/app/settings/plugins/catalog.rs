use super::*;
use crate::app::plugin_ui::management::CatalogFilter;

impl ConduitApp {
    pub(in crate::app::settings) fn settings_plugins_content(
        &self,
        view: Entity<Self>,
    ) -> AnyElement {
        let discover = self.plugins.packages.catalog.discover;
        let installed_view = view.clone();
        let discovery_view = view.clone();
        v_flex()
            .h_full()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .px_4()
                    .pt_3()
                    .pb_2()
                    .gap_2()
                    .child(
                        Button::new("plugins-installed-tab")
                            .debug_selector(|| "plugins-installed-tab".into())
                            .label(text("installed_tab"))
                            .selected(!discover)
                            .on_click(move |_, _, cx| {
                                installed_view.update(cx, |app, cx| {
                                    app.plugins.packages.catalog.discover = false;
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new("plugins-discover-tab")
                            .debug_selector(|| "plugins-discover-tab".into())
                            .label(text("discover_tab"))
                            .selected(discover)
                            .on_click(move |_, window, cx| {
                                discovery_view
                                    .update(cx, |app, cx| app.show_plugin_discovery(window, cx));
                            }),
                    ),
            )
            .child(div().flex_1().min_h_0().w_full().child(if discover {
                self.settings_plugin_catalog(view)
            } else {
                self.settings_installed_plugins_content(view)
            }))
            .into_any_element()
    }
    fn settings_plugin_catalog(&self, view: Entity<Self>) -> AnyElement {
        let state = &self.plugins.packages.catalog;
        let search_view = view.clone();
        let refresh_view = view.clone();
        let mut filters = h_flex().flex_wrap().gap_2();
        for (filter, key) in [
            (CatalogFilter::All, "all_plugins"),
            (CatalogFilter::Official, "official"),
            (CatalogFilter::Community, "community"),
        ] {
            let filter_view = view.clone();
            filters = filters.child(
                Button::new(SharedString::from(format!("plugins-filter-{key}")))
                    .label(text(key))
                    .small()
                    .ghost()
                    .selected(state.filter == filter)
                    .on_click(move |_, _, cx| {
                        filter_view.update(cx, |app, cx| {
                            app.plugins.packages.catalog.filter = filter;
                            cx.notify();
                        });
                    }),
            );
        }
        let mut list = v_flex()
            .id("plugin-catalog-list")
            .debug_selector(|| "plugin-catalog-list".into())
            .h_full()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .p_4()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .when_some(state.input.as_ref(), |row, input| {
                        row.child(
                            Input::new(input)
                                .aria_label(conduit::i18n::text_static(
                                    "settings.plugins.catalog_search",
                                ))
                                .prefix(FullIconName::Search)
                                .cleanable(true),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("plugin-catalog-search")
                                    .debug_selector(|| "plugin-catalog-search".into())
                                    .label(text("search"))
                                    .disabled(state.loading || self.closing)
                                    .on_click(move |_, _, cx| {
                                        search_view.update(cx, |app, cx| {
                                            app.query_plugin_catalog(false, cx)
                                        })
                                    }),
                            )
                            .child(
                                Button::new("plugin-catalog-refresh")
                                    .label(text("refresh"))
                                    .ghost()
                                    .disabled(state.loading || self.closing)
                                    .on_click(move |_, _, cx| {
                                        refresh_view.update(cx, |app, cx| {
                                            app.query_plugin_catalog(false, cx)
                                        })
                                    }),
                            ),
                    ),
            )
            .child(filters)
            .child(caption(text("catalog_hint")))
            .when(state.loading, |list| {
                list.child(caption(text("catalog_loading")))
            })
            .when(state.cached, |list| {
                list.child(caption(text("cached_results")))
            })
            .when(state.fetched_at > 0, |list| {
                list.child(caption(conduit::i18n::format(
                    "settings.plugins.catalog_updated",
                    &[(
                        "time",
                        chrono::DateTime::from_timestamp(state.fetched_at as i64, 0)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default(),
                    )],
                )))
            })
            .when_some(self.plugins.packages.feedback.clone(), |list, feedback| {
                list.child(caption(feedback))
            });
        if !state.errors.is_empty() {
            list = list.child(
                v_flex()
                    .gap_1()
                    .child(caption(text("catalog_failed")))
                    .children(
                        state
                            .errors
                            .iter()
                            .take(4)
                            .map(|error| caption(error.clone())),
                    ),
            );
        }
        let query = state.query.to_lowercase();
        let mut shown = 0;
        for entry in &state.entries {
            if (state.filter == CatalogFilter::Official && !entry.official())
                || (state.filter == CatalogFilter::Community && entry.official())
            {
                continue;
            }
            if entry.official()
                && !query.contains('/')
                && !query.is_empty()
                && !format!(
                    "{} {} {}",
                    entry.plugin.name, entry.plugin.description, entry.repository
                )
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            shown += 1;
            let installed = self
                .plugins
                .packages
                .entries
                .iter()
                .find(|installed| installed.package.manifest.id == entry.plugin.id);
            let compatible = entry.compatible();
            let disabled = !self.writable
                || self.closing
                || self.plugins.packages.busy
                || !compatible
                || installed.is_some();
            let install_view = view.clone();
            let install_entry = entry.clone();
            let source_url = entry.repository_url();
            let key = entry.key();
            let debug_key = entry.plugin.id.clone();
            list = list.child(
                v_flex()
                    .id(SharedString::from(format!("catalog-{key}")))
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
                            .flex_wrap()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(ui_text_size(14.))
                                    .font_medium()
                                    .child(entry.plugin.name.clone()),
                            )
                            .child(caption(entry.plugin.version.clone()))
                            .child(caption(text(if entry.official() {
                                "official"
                            } else {
                                "community"
                            }))),
                    )
                    .child(caption(entry.plugin.description.clone()))
                    .child(caption(format!(
                        "{} · {}",
                        entry.repository, entry.plugin.id
                    )))
                    .child(caption(format!(
                        "{} · {}",
                        if entry.plugin.kind == "file-icons" {
                            text("file_icons")
                        } else {
                            entry.plugin.kind.clone()
                        },
                        entry
                            .plugin
                            .license
                            .clone()
                            .unwrap_or_else(|| text("license_unspecified"))
                    )))
                    .when(!compatible, |card| {
                        card.child(caption(text("incompatible")))
                    })
                    .when(entry.bundled, |card| {
                        card.child(caption(text("bundled_available")))
                    })
                    .when_some(installed, |card, installed| {
                        card.child(caption(conduit::i18n::format(
                            "settings.plugins.installed_version",
                            &[("version", installed.package.manifest.version.clone())],
                        )))
                    })
                    .child(
                        h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new("catalog-install")
                                    .debug_selector(move || format!("catalog-install-{debug_key}"))
                                    .label(text(if installed.is_some() {
                                        "already_installed"
                                    } else {
                                        "install_online"
                                    }))
                                    .disabled(disabled)
                                    .on_click(move |_, _, cx| {
                                        install_view.update(cx, |app, cx| {
                                            app.install_catalog_plugin(install_entry.clone(), cx)
                                        });
                                    }),
                            )
                            .child(
                                Button::new("catalog-source")
                                    .label(text("view_repository"))
                                    .ghost()
                                    .on_click(move |_, _, cx| cx.open_url(&source_url)),
                            ),
                    ),
            );
        }
        if shown == 0 && !state.loading {
            list = list.child(caption(text("catalog_empty")));
        }
        if state.has_more {
            let more_view = view;
            list = list.child(
                Button::new("plugin-catalog-more")
                    .debug_selector(|| "plugin-catalog-more".into())
                    .label(text("load_more"))
                    .disabled(state.loading || self.closing)
                    .on_click(move |_, _, cx| {
                        more_view.update(cx, |app, cx| app.query_plugin_catalog(true, cx))
                    }),
            );
        }
        list.into_any_element()
    }
}
