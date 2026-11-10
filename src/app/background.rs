//! Retained wallpaper layer and explicitly opened, bounded Wallhaven browser.

use super::*;
use crate::wallpaper::{self, Category, Query, Sort, Wallpaper};
use gpui_kit::component::switch::Switch;

mod browser;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;

pub(super) struct BackgroundState {
    pub(super) layer: Entity<BackgroundLayer>,
    operation: Option<Arc<AtomicBool>>,
    loading_id: Option<String>,
    browser: Option<Entity<browser::WallpaperBrowser>>,
    error: Option<conduit::i18n::Message>,
    revision: u64,
}

impl BackgroundState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            layer: cx.new(BackgroundLayer::new),
            operation: None,
            loading_id: None,
            browser: None,
            error: None,
            revision: 0,
        }
    }
}

impl Drop for BackgroundState {
    fn drop(&mut self) {
        if let Some(cancel) = &self.operation {
            cancel.store(true, Ordering::Release);
        }
    }
}

/// Canvas has no hitbox or input handlers; the wallpaper cannot intercept application input.
pub(super) struct BackgroundLayer {
    image: Option<Arc<RenderImage>>,
    strength: u8,
}

fn release_images(images: impl IntoIterator<Item = Arc<RenderImage>>, cx: &mut App) {
    let images: Vec<_> = images.into_iter().collect();
    // Defer until the current window is back in App.windows, so its GPU atlas is included.
    if !images.is_empty() {
        cx.defer(move |cx| {
            for image in images {
                cx.drop_image(image, None);
            }
        });
    }
}

fn rendered(pixels: image::RgbaImage) -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]))
}

impl BackgroundLayer {
    fn new(cx: &mut Context<Self>) -> Self {
        cx.on_release(|layer, cx| release_images(layer.image.take(), cx))
            .detach();
        Self {
            image: None,
            strength: 15,
        }
    }

    fn replace(&mut self, image: Option<Arc<RenderImage>>, strength: u8, cx: &mut Context<Self>) {
        release_images(self.image.take(), cx);
        self.image = image;
        self.strength = strength;
        cx.notify();
    }
}

impl Render for BackgroundLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .overflow_hidden()
            .opacity(f32::from(self.strength) / 100.)
            .children(self.image.clone().map(|image| {
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let image_bounds = ObjectFit::Cover.get_bounds(bounds, image.size(0));
                        let _ = window.paint_image(
                            bounds,
                            image_bounds,
                            Corners::default(),
                            image.clone(),
                            0,
                            false,
                        );
                    },
                )
                .size_full()
            }))
    }
}

impl ConduitApp {
    pub(super) fn restore_background(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.ui.background.enabled
            && let Some(id) = self.store.ui.background.id.clone()
        {
            self.load_background(id, None, window, cx);
        }
    }

    fn load_background(
        &mut self,
        id: String,
        item: Option<Wallpaper>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.background.operation.is_some() || self.closing || (item.is_some() && !self.writable)
        {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.background.operation = Some(cancel.clone());
        self.background.loading_id = Some(id.clone());
        self.background.error = None;
        self.background.revision += 1;
        let root = wallpaper::directory(&Store::path());
        let protected = self.store.ui.background.id.clone();
        let apply = item.is_some();
        cx.spawn_in(window, async move |this, cx| {
            let ticket = cancel.clone();
            let selected = id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    match item {
                        Some(item) => {
                            wallpaper::download(&item, &root, protected.as_deref(), &ticket)
                        }
                        None => wallpaper::restore(&root, &selected, &ticket),
                    }
                })
                .await;
            let _ = this.update_in(cx, |app, _, cx| {
                app.finish_background(id, apply, &cancel, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn finish_background(
        &mut self,
        id: String,
        apply: bool,
        cancel: &Arc<AtomicBool>,
        result: anyhow::Result<image::RgbaImage>,
        cx: &mut Context<Self>,
    ) {
        if !self
            .background
            .operation
            .as_ref()
            .is_some_and(|token| Arc::ptr_eq(token, cancel))
        {
            return;
        }
        self.background.operation = None;
        self.background.loading_id = None;
        self.background.revision += 1;
        if cancel.load(Ordering::Acquire) || self.closing || (apply && !self.writable) {
            cx.notify();
            return;
        }
        match result {
            Ok(pixels) => {
                if apply {
                    self.store.ui.background.id = Some(id);
                    self.store.ui.background.enabled = true;
                    self.persist();
                }
                if self.store.ui.background.enabled {
                    let strength = self.store.ui.background.strength;
                    self.background.layer.update(cx, |layer, cx| {
                        layer.replace(Some(rendered(pixels)), strength, cx)
                    });
                    self.refresh_background_surfaces(true, cx);
                }
            }
            Err(error) => self.background.error = Some(wallpaper::error_message(&error)),
        }
        cx.notify();
    }

    fn refresh_background_surfaces(&mut self, visible: bool, cx: &mut Context<Self>) {
        if !theme::set_background_visible(visible) {
            return;
        }
        self.sidebar_view.update(cx, |_, cx| cx.notify());
        self.file_view.update(cx, |_, cx| cx.notify());
        self.timeline.update(cx, |_, cx| cx.notify());
        // Refresh only on the opaque/translucent transition, never on thumbnail arrival or strength changes.
        cx.refresh_windows();
    }

    fn set_background_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || (enabled && self.background.operation.is_some()) {
            return;
        }
        self.store.ui.background.enabled = enabled;
        self.background.error = None;
        self.background.revision += 1;
        if enabled {
            self.restore_background(window, cx);
        } else {
            if let Some(cancel) = &self.background.operation {
                cancel.store(true, Ordering::Release);
            }
            let strength = self.store.ui.background.strength;
            self.background
                .layer
                .update(cx, |layer, cx| layer.replace(None, strength, cx));
            self.refresh_background_surfaces(false, cx);
        }
        self.persist();
        cx.notify();
    }

    fn adjust_background_strength(&mut self, delta: i16, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        let strength = (i16::from(self.store.ui.background.strength) + delta).clamp(
            i16::from(wallpaper::MIN_STRENGTH),
            i16::from(wallpaper::MAX_STRENGTH),
        ) as u8;
        if strength == self.store.ui.background.strength {
            return;
        }
        self.store.ui.background.strength = strength;
        self.background.revision += 1;
        self.background.layer.update(cx, |layer, cx| {
            layer.strength = strength;
            cx.notify();
        });
        self.persist();
        cx.notify();
    }

    fn cancel_background(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.background.operation {
            cancel.store(true, Ordering::Release);
        }
        self.background.revision += 1;
        cx.notify();
    }

    fn open_background_browser(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        if self.background.browser.is_none() {
            let app = cx.weak_entity();
            self.background.browser =
                Some(cx.new(|cx| browser::WallpaperBrowser::new(app, window, cx)));
            cx.notify();
        }
    }

    fn close_background_browser(&mut self, cx: &mut Context<Self>) {
        if let Some(browser) = self.background.browser.take() {
            browser.update(cx, |browser, cx| browser.close(cx));
            cx.notify();
        }
    }

    pub(super) fn refresh_background_language(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(browser) = &self.background.browser {
            browser.update(cx, |browser, cx| browser.refresh_language(window, cx));
        }
    }

    pub(super) fn release_hidden_background_browser(&mut self, cx: &mut Context<Self>) {
        if self.background.browser.is_some()
            && (!self.settings_ui.open
                || self.settings_ui.page != 1
                || self.settings_ui.group.is_some_and(|group| group != 5)
                || !self.settings_ui.search.read(cx).value().is_empty())
        {
            self.close_background_browser(cx);
        }
    }

    pub(super) fn background_controls(&self, view: Entity<Self>) -> AnyElement {
        let disabled = !self.writable || self.closing;
        let enabled = self.store.ui.background.enabled;
        let busy = self.background.operation.is_some();
        let expanded = self.background.browser.is_some();
        let strength = self.store.ui.background.strength;
        let toggle = view.clone();
        let browse = view.clone();
        let less = view.clone();
        let more = view.clone();
        let retry = view.clone();
        let cancel = view;
        v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static("wallpaper.description")),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(
                        Switch::new("background-toggle")
                            .label(conduit::i18n::text_static("wallpaper.enable"))
                            .checked(enabled)
                            .disabled(disabled || (busy && !enabled))
                            .on_click(move |value, window, cx| {
                                toggle.update(cx, |app, cx| {
                                    app.set_background_enabled(*value, window, cx)
                                })
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("background-browse")
                            .debug_selector(|| "background-browse".into())
                            .label(conduit::i18n::text_static(if expanded {
                                "wallpaper.collapse"
                            } else {
                                "wallpaper.browse"
                            }))
                            .icon(if expanded {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .outline()
                            .disabled(disabled)
                            .on_click(move |_, window, cx| {
                                browse.update(cx, |app, cx| {
                                    if app.background.browser.is_some() {
                                        app.close_background_browser(cx);
                                    } else {
                                        app.open_background_browser(window, cx);
                                    }
                                })
                            }),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .child(conduit::i18n::text_static("wallpaper.strength")),
                    )
                    .child(
                        Button::new("background-less")
                            .icon(IconName::Minus)
                            .small()
                            .ghost()
                            .accessibility_label(conduit::i18n::text_static("wallpaper.less"))
                            .disabled(disabled || strength <= wallpaper::MIN_STRENGTH)
                            .on_click(move |_, _, cx| {
                                less.update(cx, |app, cx| app.adjust_background_strength(-5, cx))
                            }),
                    )
                    .child(div().w(px(42.)).text_center().child(format!("{strength}%")))
                    .child(
                        Button::new("background-more")
                            .icon(IconName::Plus)
                            .small()
                            .ghost()
                            .accessibility_label(conduit::i18n::text_static("wallpaper.more"))
                            .disabled(disabled || strength >= wallpaper::MAX_STRENGTH)
                            .on_click(move |_, _, cx| {
                                more.update(cx, |app, cx| app.adjust_background_strength(5, cx))
                            }),
                    ),
            )
            .when(enabled && self.store.ui.background.id.is_none(), |body| {
                body.child(
                    div()
                        .text_size(ui_text_size(12.))
                        .text_color(rgb(palette().muted))
                        .child(conduit::i18n::text_static("wallpaper.choose_first")),
                )
            })
            .when(busy && !expanded, |body| {
                body.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(conduit::i18n::text_static("wallpaper.downloading"))
                        .child(
                            Button::new("background-cancel")
                                .small()
                                .ghost()
                                .label(conduit::i18n::text_static("wallpaper.cancel"))
                                .on_click(move |_, _, cx| {
                                    cancel.update(cx, |app, cx| app.cancel_background(cx))
                                }),
                        ),
                )
            })
            .when_some(
                self.background.error.clone().filter(|_| !expanded),
                |body, error| {
                    body.child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(ui_text_size(12.))
                                    .text_color(rgb(palette().error_text))
                                    .child(conduit::i18n::format(
                                        "wallpaper.failed",
                                        &[("error", error.render())],
                                    )),
                            )
                            .when(enabled && !busy, |body| {
                                body.child(
                                    Button::new("background-retry")
                                        .small()
                                        .ghost()
                                        .label(conduit::i18n::text_static("wallpaper.retry"))
                                        .disabled(disabled)
                                        .on_click(move |_, window, cx| {
                                            retry.update(cx, |app, cx| {
                                                app.restore_background(window, cx)
                                            })
                                        }),
                                )
                            }),
                    )
                },
            )
            .when_some(self.background.browser.clone(), |body, browser| {
                body.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .pt_3()
                        .border_t_1()
                        .border_color(rgb(palette().border))
                        .child(browser),
                )
            })
            .into_any_element()
    }
}
