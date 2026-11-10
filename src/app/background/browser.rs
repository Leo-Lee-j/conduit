use super::*;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;
mod view;

static BROWSING: AtomicBool = AtomicBool::new(false);
struct BrowsePermit;
impl Drop for BrowsePermit {
    fn drop(&mut self) {
        BROWSING.store(false, Ordering::Release);
    }
}

struct Tile {
    item: Wallpaper,
    image: Option<Arc<RenderImage>>,
    failed: bool,
}

pub(super) struct WallpaperBrowser {
    app: WeakEntity<ConduitApp>,
    input: Entity<InputState>,
    category: Category,
    sort: Sort,
    color: Option<String>,
    solid_only: bool,
    columns: u16,
    closed: bool,
    query: Option<Query>,
    last_page: u32,
    tiles: Vec<Tile>,
    cancel: Option<Arc<AtomicBool>>,
    error: Option<conduit::i18n::Message>,
    observed: (u64, bool, bool),
    _subscription: Subscription,
    _input_subscription: Subscription,
}

impl WallpaperBrowser {
    pub(super) fn new(
        app: WeakEntity<ConduitApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&app.upgrade().expect("live app"), |this, app, cx| {
            let app = app.read(cx);
            let state = (app.background.revision, app.writable, app.closing);
            if this.observed != state {
                this.observed = state;
                cx.notify();
            }
        });
        cx.on_release(|this, cx| {
            if let Some(cancel) = &this.cancel {
                cancel.store(true, Ordering::Release);
            }
            release_images(this.tiles.drain(..).filter_map(|tile| tile.image), cx);
        })
        .detach();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(conduit::i18n::text_static("wallpaper.query_hint"))
        });
        let input_subscription = cx.subscribe(&input, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.fresh_search(cx);
            }
        });
        Self {
            app,
            input,
            category: Category::All,
            sort: Sort::Popular,
            color: None,
            solid_only: false,
            columns: 1,
            closed: false,
            query: None,
            last_page: 0,
            tiles: Vec::new(),
            cancel: None,
            error: None,
            observed: (0, true, false),
            _subscription: subscription,
            _input_subscription: input_subscription,
        }
    }

    pub(super) fn refresh_language(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_placeholder(
                conduit::i18n::text_static("wallpaper.query_hint"),
                window,
                cx,
            );
        });
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        release_images(self.tiles.drain(..).filter_map(|tile| tile.image), cx);
        self.query = None;
        self.last_page = 0;
    }

    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        self.closed = true;
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Release);
        }
        self.clear(cx);
    }

    fn fresh_search(&mut self, cx: &mut Context<Self>) {
        self.search(
            Query {
                text: self.input.read(cx).value().to_string(),
                category: self.category,
                sort: self.sort,
                page: 1,
                seed: None,
                color: self.color.clone(),
                solid_only: self.solid_only,
            },
            cx,
        );
    }

    fn search(&mut self, query: Query, cx: &mut Context<Self>) {
        if self.closed
            || self.cancel.is_some()
            || self.app.upgrade().is_none_or(|app| {
                let app = app.read(cx);
                !app.writable || app.closing
            })
        {
            return;
        }
        if BROWSING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            self.error = Some(conduit::i18n::Message::new("wallpaper.wait_cancel", &[]));
            cx.notify();
            return;
        }
        let permit = BrowsePermit;
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = Some(cancel.clone());
        self.error = None;
        self.clear(cx);
        cx.spawn(async move |this, cx| {
            // This detached task keeps the permit until an in-flight synchronous operation finishes,
            // even if the inline browser closes and is opened again immediately.
            let _permit = permit;
            let ticket = cancel.clone();
            let request = query.clone();
            let result = cx
                .background_executor()
                .spawn(async move { wallpaper::search(&request, &ticket) })
                .await;
            let items = match result {
                Ok(page) => {
                    let items = page.data.clone();
                    let accepted = this
                        .update(cx, |this, cx| {
                            if !this.accepts(&cancel) {
                                return false;
                            }
                            this.last_page = page.meta.last_page.min(10_000);
                            this.query = Some(Query {
                                seed: page.meta.seed,
                                ..query
                            });
                            this.tiles = page
                                .data
                                .into_iter()
                                .map(|item| Tile {
                                    item,
                                    image: None,
                                    failed: false,
                                })
                                .collect();
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !accepted {
                        Self::finish(&this, &cancel, None, cx);
                        return;
                    }
                    items
                }
                Err(error) => {
                    Self::finish(&this, &cancel, Some(wallpaper::error_message(&error)), cx);
                    return;
                }
            };
            // One bounded batch at a time. At most four thumbnail results cross the UI boundary.
            let deadline = Instant::now() + Duration::from_secs(45);
            for (batch_index, batch) in items.chunks(4).enumerate() {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let batch = batch.to_vec();
                let ticket = cancel.clone();
                let images = cx
                    .background_executor()
                    .spawn(async move {
                        batch
                            .iter()
                            .map(|item| {
                                if Instant::now() >= deadline {
                                    anyhow::bail!("Thumbnail loading timed out");
                                }
                                wallpaper::thumbnail(item, &ticket).map(rendered)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let accepted = this
                    .update(cx, |this, cx| {
                        if !this.accepts(&cancel) {
                            return false;
                        }
                        for (offset, result) in images.into_iter().enumerate() {
                            if let Some(tile) = this.tiles.get_mut(batch_index * 4 + offset) {
                                tile.failed = result.is_err();
                                tile.image = result.ok();
                            }
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !accepted {
                    break;
                }
            }
            Self::finish(&this, &cancel, None, cx);
        })
        .detach();
        cx.notify();
    }

    fn accepts(&self, cancel: &Arc<AtomicBool>) -> bool {
        !cancel.load(Ordering::Acquire)
            && self
                .cancel
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, cancel))
    }

    fn finish(
        this: &WeakEntity<Self>,
        cancel: &Arc<AtomicBool>,
        error: Option<conduit::i18n::Message>,
        cx: &mut AsyncApp,
    ) {
        let _ = this.update(cx, |this, cx| {
            if this
                .cancel
                .as_ref()
                .is_some_and(|active| Arc::ptr_eq(active, cancel))
            {
                if !cancel.load(Ordering::Acquire) {
                    this.error = error;
                }
                this.cancel = None;
                for tile in &mut this.tiles {
                    if tile.image.is_none() {
                        tile.failed = true;
                    }
                }
                cx.notify();
            }
        });
    }
}
