//! Wheel movement uses message anchors, never a sum with zero-height unknown rows.
use super::*;

fn same(a: ListOffset, b: ListOffset) -> bool {
    a.item_ix == b.item_ix && a.offset_in_item == b.offset_in_item
}

fn start() -> ListOffset {
    ListOffset {
        item_ix: 0,
        offset_in_item: px(0.),
    }
}

fn viewport_top(scroll: &ListState) -> ListOffset {
    let mut top = ListOffset {
        item_ix: scroll.item_count(),
        offset_in_item: px(0.),
    };
    let mut remaining = scroll.viewport_bounds().size.height;
    scroll.scroll_to(start());
    while top.item_ix > 0 && remaining > px(0.) {
        let Some(bounds) = scroll.bounds_for_item(top.item_ix - 1) else {
            break;
        };
        top.item_ix -= 1;
        remaining -= bounds.size.height;
    }
    top.offset_in_item = (-remaining).max(px(0.));
    top
}

struct Pending {
    target: ListOffset,
    displayed: ListOffset,
}

#[derive(Default)]
pub(super) struct Wheel {
    pending: Option<Pending>,
    context: Option<(Option<u64>, u64)>,
}

impl Wheel {
    pub(super) fn is_pending(&self, app: &ConduitApp) -> bool {
        self.context == Some((app.selected_session, app.chat_rows.revision.get()))
            && app.pending_message_jump.is_none()
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| same(pending.displayed, app.scroll.logical_scroll_top()))
    }

    // A session switch or a rebuilt row map can reuse the same numeric offset.
    // Never carry a deferred gesture into that new transcript.
    pub(super) fn sync(&mut self, app: &ConduitApp) {
        let context = (app.selected_session, app.chat_rows.revision.get());
        if self.context != Some(context) || app.pending_message_jump.is_some() {
            self.pending = None;
            self.context = Some(context);
        }
    }

    /// Return whether a downward packet leaves an already-settled bottom alone.
    pub(super) fn apply(&mut self, scroll: &ListState, origin: ListOffset, delta: Pixels) -> bool {
        let pending = self.pending.take().filter(|p| same(p.displayed, origin));
        // The built-in handler has already changed the list. Test the captured
        // anchor, not that handler's result or an unresolved wheel target.
        scroll.scroll_to(origin);
        if pending.is_none()
            && delta < px(0.)
            && (origin.item_ix >= scroll.item_count() || scroll.is_scrolled_to_end() == Some(true))
        {
            // Discard overscroll instead of toggling follow off and back on.
            // That toggle inserts the latest-message row above the composer,
            // resizing the transcript for one frame on every wheel packet.
            scroll.scroll_to_end();
            return true;
        }
        // Restore the valid viewport anchor after the built-in list handler.
        // Keep the gesture separate until layout can resolve every crossed row.
        let displayed = if origin.item_ix >= scroll.item_count() {
            viewport_top(scroll)
        } else {
            origin
        };
        let mut top = pending.map_or(displayed, |p| p.target);
        top.offset_in_item -= delta;
        if top.item_ix == 0 {
            top.offset_in_item = top.offset_in_item.max(px(0.));
        }
        scroll.scroll_to(displayed);
        self.pending = Some(Pending {
            target: top,
            displayed,
        });
        false
    }

    pub(super) fn prepare(
        &mut self,
        scroll: &ListState,
        app: &WeakEntity<ConduitApp>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(app) = app.upgrade() {
            self.sync(app.read(cx));
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        if !same(pending.displayed, scroll.logical_scroll_top()) {
            return;
        }
        let mut top = pending.target;
        let mut waiting = false;
        while top.offset_in_item < px(0.) && top.item_ix > 0 {
            let Some(height) = measure(app, top.item_ix - 1, bounds.size.width, window, cx) else {
                waiting = true;
                break;
            };
            top.item_ix -= 1;
            top.offset_in_item += height;
        }
        if top.item_ix == 0 {
            top.offset_in_item = top.offset_in_item.max(px(0.));
        }
        while !waiting && top.item_ix < scroll.item_count() && top.offset_in_item > px(0.) {
            let Some(height) = measure(app, top.item_ix, bounds.size.width, window, cx) else {
                waiting = true;
                break;
            };
            if top.offset_in_item < height {
                break;
            }
            top.offset_in_item -= height;
            top.item_ix += 1;
        }
        if waiting {
            // The keyed Markdown state notifies the timeline when its parser
            // finishes. Keep the remaining distance, but present a valid row
            // anchor in the meantime; negative offsets leave a blank viewport.
            let displayed = ListOffset {
                item_ix: top.item_ix,
                offset_in_item: px(0.),
            };
            self.pending = Some(Pending {
                target: top,
                displayed,
            });
            scroll.scroll_to(displayed);
        } else {
            scroll.scroll_to(top);
        }
        // Row crossings now happen during layout, after the wheel listener.
        // Refresh the outline only when its active section actually changes.
        let _ = app.update(cx, |app, cx| {
            let top = scroll.logical_scroll_top();
            let active = app
                .outline
                .active_for_item(app.chat_rows.message(top.item_ix));
            if app.outline.active != active {
                cx.notify();
            }
        });
    }
}

fn measure(
    app: &WeakEntity<ConduitApp>,
    index: usize,
    width: Pixels,
    window: &mut Window,
    cx: &mut App,
) -> Option<Pixels> {
    let (mut element, markdown, text) = app
        .update(cx, |app, cx| {
            let text = app
                .selected_session
                .and_then(|id| app.session(id))
                .and_then(|session| session.messages.get(app.chat_rows.message(index)))
                .filter(|item| item.kind == ItemKind::Assistant)
                .map(|item| item.text.clone());
            let (element, markdown) = app.render_message(index, window, cx);
            (element, markdown, text)
        })
        .ok()?;
    let height = element
        .layout_as_root(
            size(AvailableSpace::Definite(width), AvailableSpace::MinContent),
            window,
            cx,
        )
        .height;
    // Large Markdown documents parse asynchronously, initially laying out as
    // an empty element. That height must never consume the user's wheel delta.
    if let (Some(markdown), Some(text)) = (markdown, text)
        && markdown.read(cx).rendered_text().source() != text
    {
        return None;
    }
    Some(height)
}
