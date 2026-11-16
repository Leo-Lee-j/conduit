use super::*;

impl WallpaperBrowser {
    fn choices<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        selected: T,
        disabled: bool,
        change: fn(&mut Self, T),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .w_full()
            .gap_1()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static(id)),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .children(options.iter().copied().enumerate().map(
                        |(index, (value, label))| {
                            Button::new((id, index))
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static(label))
                                .selected(selected == value)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    change(this, value);
                                    this.error = None;
                                    cx.notify();
                                }))
                        },
                    )),
            )
            .into_any_element()
    }

    pub(super) fn filters(&self, disabled: bool, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .w_full()
            .gap_2()
            .child(self.choices(
                "wallpaper.category",
                &[
                    (Category::All, "wallpaper.all"),
                    (Category::General, "wallpaper.general"),
                    (Category::Anime, "wallpaper.anime"),
                    (Category::People, "wallpaper.people"),
                ],
                self.category,
                disabled,
                |this, value| this.category = value,
                cx,
            ))
            .child(self.choices(
                "wallpaper.sort",
                &[
                    (Sort::Popular, "wallpaper.popular"),
                    (Sort::Latest, "wallpaper.latest"),
                    (Sort::Random, "wallpaper.random"),
                    (Sort::Views, "wallpaper.views"),
                    (Sort::Favorites, "wallpaper.favorites"),
                ],
                self.sort,
                disabled,
                |this, value| this.sort = value,
                cx,
            ))
            .when(
                self.advanced_filters && self.sort == Sort::Popular,
                |body| {
                    body.child(self.choices(
                        "wallpaper.top_range",
                        &[
                            (TopRange::Day, "wallpaper.day"),
                            (TopRange::Week, "wallpaper.week"),
                            (TopRange::Month, "wallpaper.month"),
                            (TopRange::HalfYear, "wallpaper.half_year"),
                            (TopRange::Year, "wallpaper.year"),
                        ],
                        self.top_range,
                        disabled,
                        |this, value| this.top_range = value,
                        cx,
                    ))
                },
            )
            .when(self.advanced_filters, |body| {
                body.child(self.choices(
                    "wallpaper.resolution",
                    &[
                        (Resolution::Any, "wallpaper.any"),
                        (Resolution::FullHd, "wallpaper.full_hd"),
                        (Resolution::Qhd, "wallpaper.qhd"),
                        (Resolution::UltraHd, "wallpaper.ultra_hd"),
                    ],
                    self.resolution,
                    disabled,
                    |this, value| this.resolution = value,
                    cx,
                ))
                .child(self.choices(
                    "wallpaper.ratio",
                    &[
                        (Ratio::Any, "wallpaper.any"),
                        (Ratio::Wide, "wallpaper.wide"),
                        (Ratio::Ultrawide, "wallpaper.ultrawide"),
                        (Ratio::Portrait, "wallpaper.portrait"),
                        (Ratio::Square, "wallpaper.square"),
                    ],
                    self.ratio,
                    disabled,
                    |this, value| this.ratio = value,
                    cx,
                ))
            })
            .into_any_element()
    }
}
