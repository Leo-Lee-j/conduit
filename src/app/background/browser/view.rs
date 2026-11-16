use super::*;
use gpui_kit::component::{progress::Progress, tag::Tag};

impl WallpaperBrowser {
    fn colors(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let mut swatches = h_flex().flex_wrap().gap_2().items_center().child(
            Button::new("wallpaper-color-any")
                .small()
                .outline()
                .label(conduit::i18n::text_static("wallpaper.any_color"))
                .selected(self.color.is_none())
                .disabled(disabled)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.color = None;
                    this.error = None;
                    cx.notify();
                })),
        );
        for (index, &color) in wallpaper::COLORS.iter().enumerate() {
            let value = u32::from_str_radix(color, 16).expect("fixed Wallhaven palette");
            let selected = self.color.as_deref() == Some(color);
            let light =
                ((value >> 16) * 299 + ((value >> 8) & 255) * 587 + (value & 255) * 114) > 150_000;
            swatches =
                swatches.child(
                    Button::new(("wallpaper-color", index))
                        .outline()
                        .size(px(28.))
                        .p(px(3.))
                        .disabled(disabled)
                        .selected(selected)
                        .tooltip(format!("#{color}"))
                        .accessibility_label(conduit::i18n::format(
                            "wallpaper.color_id",
                            &[("color", format!("#{color}"))],
                        ))
                        .child(
                            div()
                                .size_full()
                                .rounded_sm()
                                .bg(rgb(value))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(selected, |swatch| {
                                    swatch.child(
                                        Icon::new(IconName::Check).small().text_color(rgb(
                                            if light { 0x000000 } else { 0xffffff },
                                        )),
                                    )
                                }),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.color = if selected { None } else { Some(color.into()) };
                            this.error = None;
                            cx.notify();
                        })),
                );
        }
        v_flex()
            .gap_2()
            .w_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .child(conduit::i18n::text_static("wallpaper.color")),
                    )
                    .when_some(self.color.clone(), |row, color| {
                        row.child(
                            div()
                                .text_size(ui_text_size(12.))
                                .text_color(rgb(palette().muted))
                                .child(format!("#{color}")),
                        )
                    }),
            )
            .child(swatches)
            .child(
                Switch::new("wallpaper-solid-only")
                    .label(conduit::i18n::text_static("wallpaper.solid_only"))
                    .checked(self.solid_only)
                    .disabled(disabled)
                    .on_click(cx.listener(|this, value, _, cx| {
                        this.solid_only = *value;
                        this.error = None;
                        cx.notify();
                    })),
            )
            .when(self.solid_only, |body| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("wallpaper.solid_hint")),
                )
            })
            .into_any_element()
    }

    fn tile(&self, index: usize, tile: &Tile, cx: &App) -> AnyElement {
        let entry = tile.entry.clone();
        let (selected, loading, disabled) = self
            .app
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                (
                    app.store.ui.background.enabled
                        && app.background.layer.read(cx).image.is_some()
                        && app.store.ui.background.id.as_deref() == Some(&entry.id),
                    app.background.loading_id.as_deref() == Some(&entry.id),
                    !app.writable || app.closing || app.background.operation.is_some(),
                )
            })
            .unwrap_or((false, false, true));
        let app = self.app.clone();
        let mut preview = div()
            .relative()
            .w_full()
            .h(px(132.))
            .overflow_hidden()
            .rounded_md()
            .bg(rgb(palette().surface_strong));
        if let Some(image) = &tile.image {
            preview = preview.child(
                gpui_kit::img(image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Cover),
            );
        } else {
            preview = preview
                .flex()
                .items_center()
                .justify_center()
                .text_size(ui_text_size(12.))
                .child(conduit::i18n::text_static(if tile.failed {
                    "wallpaper.preview_failed"
                } else {
                    "wallpaper.preview_loading"
                }));
        }
        preview = preview.when_some(entry.dimensions(), |preview, (width, height)| {
            preview.child(
                div().absolute().top_2().right_2().child(
                    Tag::secondary()
                        .small()
                        .text_size(ui_text_size(11.))
                        .child(format!("{width} × {height}")),
                ),
            )
        });
        v_flex()
            .min_w_0()
            .w_full()
            .p_2()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if selected {
                palette().activity_running
            } else {
                palette().border
            }))
            .bg(rgb(palette().surface))
            .child(preview)
            .child(
                Button::new(("wallpaper-apply", index))
                    .debug_selector(move || format!("wallpaper-apply-{index}"))
                    .outline()
                    .w_full()
                    .h(px(32.))
                    .py(px(3.))
                    .selected(selected)
                    .disabled(disabled || selected)
                    .when(selected, |button| button.icon(IconName::Check))
                    .when(!loading, |button| {
                        button.label(conduit::i18n::text_static(if selected {
                            "wallpaper.selected"
                        } else {
                            "wallpaper.apply"
                        }))
                    })
                    .when(loading, |button| {
                        button.child(
                            v_flex()
                                .w_full()
                                .min_w_0()
                                .gap(px(2.))
                                .items_center()
                                .child(
                                    div()
                                        .text_size(ui_text_size(12.))
                                        .line_height(px(16.))
                                        .child(conduit::i18n::text_static("wallpaper.applying")),
                                )
                                .child(
                                    Progress::new(("wallpaper-apply-progress", index))
                                        .loading(true)
                                        .xsmall()
                                        .w_full()
                                        .flex_shrink_0()
                                        .accessibility_label(conduit::i18n::text_static(
                                            "wallpaper.applying",
                                        )),
                                ),
                        )
                    })
                    .accessibility_label(conduit::i18n::format(
                        "wallpaper.apply_id",
                        &[(
                            "id",
                            entry
                                .local
                                .as_ref()
                                .map(|local| local.name.clone())
                                .unwrap_or_else(|| entry.id.clone()),
                        )],
                    ))
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |app, cx| {
                            app.load_background(entry.clone(), true, window, cx)
                        });
                    }),
            )
            .into_any_element()
    }
}

impl Render for WallpaperBrowser {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.cancel.is_some();
        let (applying, disabled, apply_error) = self
            .app
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                (
                    app.background.operation.is_some(),
                    !app.writable || app.closing,
                    app.background.error.clone(),
                )
            })
            .unwrap_or((false, true, None));
        let changed = self.filters_changed(cx);
        let tiles = if self.showing_history {
            &self.history_tiles
        } else {
            &self.tiles
        };
        let page = self.query.as_ref().map_or(0, |q| q.page);
        let previous = self.query.clone();
        let next = self.query.clone();
        let measure = cx.weak_entity();
        let columns = self.columns;
        v_flex()
            .id("wallpaper-browser")
            .w_full()
            .min_w_0()
            .gap_3()
            // Measure the settings block, which may be narrower than the application window.
            // Only a column-count change schedules a render; no continuous resize notifications.
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let count = ((f32::from(bounds.size.width) + 12.) / 204.)
                            .floor()
                            .clamp(1., 4.) as u16;
                        if count != columns {
                            let measure = measure.clone();
                            cx.defer(move |cx| {
                                let _ = measure.update(cx, |this, cx| {
                                    if this.columns != count {
                                        this.columns = count;
                                        cx.notify();
                                    }
                                });
                            });
                        }
                    },
                    |_, _, _, _| {},
                )
                .w_full()
                .h(px(0.)),
            )
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static("wallpaper.browser_hint")),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("wallpaper-search-tab")
                            .small()
                            .ghost()
                            .label(conduit::i18n::text_static("wallpaper.discover"))
                            .selected(!self.showing_history)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.showing_history = false;
                                this.error = None;
                                release_images(
                                    this.history_tiles.drain(..).filter_map(|tile| tile.image),
                                    cx,
                                );
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("wallpaper-history-tab")
                            .small()
                            .ghost()
                            .debug_selector(|| "wallpaper-history-tab".into())
                            .label(conduit::i18n::text_static("wallpaper.history"))
                            .selected(self.showing_history)
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.load_history(cx))),
                    ),
            )
            .when(!self.showing_history, |body| {
                body.child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .child(conduit::i18n::text_static("wallpaper.query_label")),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .flex_wrap()
                                .gap_2()
                                .items_center()
                                .child(
                                    div().flex_1().min_w(px(160.)).child(
                                        Input::new(&self.input)
                                            .disabled(busy || disabled)
                                            .h(px(36.))
                                            .text_size(ui_text_size(14.)),
                                    ),
                                )
                                .child(
                                    Button::new("wallpaper-search")
                                        .debug_selector(|| "wallpaper-search".into())
                                        .primary()
                                        .h(px(36.))
                                        .px_4()
                                        .text_size(ui_text_size(14.))
                                        .label(conduit::i18n::text_static("wallpaper.search"))
                                        .disabled(busy || disabled)
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.fresh_search(cx)),
                                        ),
                                ),
                        ),
                )
                .child(self.filters(busy || disabled, cx))
                .when(self.advanced_filters, |body| {
                    body.child(self.colors(busy || disabled, cx))
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            Button::new("wallpaper-advanced")
                                .small()
                                .ghost()
                                .debug_selector(|| "wallpaper-advanced".into())
                                .label(conduit::i18n::text_static(if self.advanced_filters {
                                    "wallpaper.fewer_filters"
                                } else {
                                    "wallpaper.more_filters"
                                }))
                                .selected(
                                    self.advanced_filters
                                        || self.color.is_some()
                                        || self.solid_only
                                        || self.resolution != Resolution::default()
                                        || self.ratio != Ratio::default()
                                        || (self.sort == Sort::Popular
                                            && self.top_range != TopRange::default()),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.advanced_filters = !this.advanced_filters;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("wallpaper-reset")
                                .small()
                                .ghost()
                                .debug_selector(|| "wallpaper-reset".into())
                                .label(conduit::i18n::text_static("wallpaper.reset_filters"))
                                .disabled(busy || disabled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reset_filters(window, cx)
                                })),
                        )
                        .when(changed, |row| {
                            row.child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::text_static("wallpaper.filters_changed")),
                            )
                        }),
                )
                .child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("wallpaper.query_help")),
                )
            })
            .when(self.showing_history, |body| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("wallpaper.history_hint")),
                )
            })
            .when(busy, |body| {
                body.child(
                    h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .child(conduit::i18n::text_static("wallpaper.loading")),
                        )
                        .child(
                            Button::new("wallpaper-cancel")
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("wallpaper.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(cancel) = &this.cancel {
                                        cancel.store(true, Ordering::Release);
                                    }
                                    cx.notify();
                                })),
                        ),
                )
            })
            .when(applying, |body| {
                body.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_size(ui_text_size(12.))
                                .child(conduit::i18n::text_static("wallpaper.downloading")),
                        )
                        .child(
                            Button::new("wallpaper-download-cancel")
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("wallpaper.cancel"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let _ =
                                        this.app.update(cx, |app, cx| app.cancel_background(cx));
                                })),
                        ),
                )
            })
            .when_some(self.error.clone().or(apply_error), |body, error| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().error_text))
                        .child(conduit::i18n::format(
                            "wallpaper.failed",
                            &[("error", error.render())],
                        )),
                )
            })
            // Natural height shares the settings scroller; no nested wheel trap.
            .child(
                div()
                    .id("wallpaper-results")
                    .w_full()
                    .min_w_0()
                    .when(tiles.is_empty() && !busy, |body| {
                        body.child(
                            div()
                                .py_6()
                                .text_size(ui_text_size(13.))
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static(if self.showing_history {
                                    "wallpaper.history_empty"
                                } else if self.query.is_some() {
                                    "wallpaper.empty"
                                } else {
                                    "wallpaper.start"
                                })),
                        )
                    })
                    .child(
                        div()
                            .grid()
                            .grid_cols(self.columns)
                            .w_full()
                            .min_w_0()
                            .gap_3()
                            .children(
                                tiles
                                    .iter()
                                    .enumerate()
                                    .map(|(index, tile)| self.tile(index, tile, cx)),
                            ),
                    ),
            )
            .when(!self.showing_history && self.query.is_some(), |body| {
                body.child(
                    h_flex()
                        .flex_wrap()
                        .gap_3()
                        .items_center()
                        .child(
                            Button::new("wallpaper-previous")
                                .outline()
                                .label(conduit::i18n::text_static("wallpaper.previous"))
                                .disabled(busy || disabled || changed || page <= 1)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(mut query) = previous.clone() {
                                        query.page = query.page.saturating_sub(1).max(1);
                                        this.search(query, cx);
                                    }
                                })),
                        )
                        .child(
                            div()
                                .text_size(ui_text_size(13.))
                                .child(conduit::i18n::format(
                                    "wallpaper.page",
                                    &[
                                        ("page", page.to_string()),
                                        ("total", self.last_page.max(1).to_string()),
                                    ],
                                )),
                        )
                        .child(
                            Button::new("wallpaper-next")
                                .outline()
                                .label(conduit::i18n::text_static("wallpaper.next"))
                                .disabled(
                                    busy || disabled
                                        || changed
                                        || page == 0
                                        || page >= self.last_page,
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(mut query) = next.clone() {
                                        query.page += 1;
                                        this.search(query, cx);
                                    }
                                })),
                        ),
                )
            })
    }
}
