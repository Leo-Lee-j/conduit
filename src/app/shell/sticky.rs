//! Compact prompt navigation using this frame of visible transcript rows.
use super::*;
use crate::app::chat::{CHAT_CONTENT_GUTTER, CHAT_CONTENT_MAX_WIDTH};
use std::{cell::RefCell, rc::Rc};

type RowBounds = (usize, Bounds<Pixels>);

/// Follow-tail uses an end sentinel, not the first visible row. Observe actual
/// prepaint bounds without changing the scroll anchor or follow intent.
#[derive(Clone, Default)]
pub(super) struct Frame(Rc<RefCell<Vec<RowBounds>>>);

impl Frame {
    pub(super) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub(super) fn track(&self, index: usize, row: AnyElement) -> AnyElement {
        let rows = self.0.clone();
        div()
            .relative()
            .w_full()
            .child(row)
            .child(
                canvas(
                    move |bounds, _, _| rows.borrow_mut().push((index, bounds)),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .into_any_element()
    }

    fn prompt(
        &self,
        source: &TimelineSource,
        app: &ConduitApp,
        scroll: &ListState,
    ) -> Option<(u64, usize, outline::MessageOutlineEntry)> {
        let viewport = scroll.viewport_bounds();
        if viewport.size.height <= px(0.) {
            return None;
        }
        let rows = self.0.borrow();
        let &(row, bounds) = rows
            .iter()
            .filter(|(_, bounds)| {
                bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom()
            })
            .min_by_key(|(index, _)| *index)?;
        let index = source.message(app, row);
        let session = source.id(app)?;
        source.with_outline(app, |outline| {
            if outline.session != Some(session) {
                return None;
            }
            let ordinal = outline
                .entries
                .partition_point(|entry| entry.index <= index)
                .checked_sub(1)?;
            let entry = &outline.entries[ordinal];
            // No duplicate while the original prompt starts inside the viewport.
            if entry.index == index && bounds.top() >= viewport.top() - px(0.5) {
                return None;
            }
            Some((session, ordinal, entry.clone()))
        })
    }

    pub(super) fn header(
        self,
        app: WeakEntity<ConduitApp>,
        source: TimelineSource,
        scroll: ListState,
        height: Pixels,
    ) -> AnyElement {
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(height)
            .px(px(CHAT_CONTENT_GUTTER))
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, window, cx| {
                        // This canvas follows the list, so its geometry includes
                        // the wheel movement resolved during the current layout.
                        let mut header = app
                            .update(cx, |app, cx| {
                                let (session, ordinal, entry) =
                                    self.prompt(&source, app, &scroll)?;
                                let label = conduit::i18n::format(
                                    "outline.jump_to_user_message",
                                    &[
                                        ("arg0", format!("{}", ordinal + 1)),
                                        ("arg1", entry.label.clone()),
                                    ],
                                );
                                let key = entry.key.clone();
                                let source = source.clone();
                                Some(
                                    div()
                                        .relative()
                                        .w_full()
                                        .h_full()
                                        .when(!app.store.ui.full_width_content, |row| {
                                            row.max_w(px(CHAT_CONTENT_MAX_WIDTH)).mx_auto()
                                        })
                                        .child(header_background(
                                            cx.theme().radius_lg.min(height / 3.),
                                        ))
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "sticky-prompt-{session}-{key}"
                                            )))
                                            .debug_selector(move || {
                                                format!("sticky-prompt-{session}-{}", entry.key)
                                            })
                                            .text()
                                            .w_full()
                                            .h(height)
                                            .px_2()
                                            .py_0()
                                            .rounded(px(6.))
                                            .rounded_tl(px(0.))
                                            .rounded_tr(px(0.))
                                            .text_size(ui_text_size(13.))
                                            .text_color(rgb(palette().muted))
                                            .tooltip(label.clone())
                                            .accessibility_label(label)
                                            .child(
                                                h_flex()
                                                    .w_full()
                                                    .min_w_0()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .text_left()
                                                            .truncate()
                                                            .child(entry.label),
                                                    )
                                                    .child(
                                                        Icon::new(IconName::ArrowUp)
                                                            .size(px(12.))
                                                            .flex_shrink_0(),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |app, _, _, cx| {
                                                source.jump_to_outline_message(
                                                    session, &key, app, cx,
                                                );
                                            })),
                                        )
                                        .into_any_element(),
                                )
                            })
                            .ok()
                            .flatten()?;
                        header.layout_as_root(bounds.size.into(), window, cx);
                        header.prepaint_at(bounds.origin, window, cx);
                        Some(header)
                    },
                    |_, header, window, cx| {
                        if let Some(mut header) = header {
                            header.paint(window, cx);
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

/// Mirror the folder tab contour: the upper shoulders join the top edge,
/// while the lower corners remain rounded. Paint a single fill behind the
/// transparent button so its hover state cannot split the shoulders from it.
fn header_background(curve: Pixels) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (top, bottom) = (bounds.top(), bounds.bottom());
            let (left, right) = (bounds.left(), bounds.right());
            let (body_left, body_right) = (left + curve, right - curve);
            let radius =
                ((body_right - body_left) / 3.).clamp(px(0.), px(6.).min(bounds.size.height / 2.));
            let mut path =
                PathBuilder::fill().with_style(PathStyle::Fill(FillOptions::tolerance(0.02)));
            path.move_to(point(left, top));
            quarter_circle_to(
                &mut path,
                point(left, top),
                point(body_left, top),
                point(body_left, top + curve),
            );
            path.line_to(point(body_left, bottom - radius));
            quarter_circle_to(
                &mut path,
                point(body_left, bottom - radius),
                point(body_left, bottom),
                point(body_left + radius, bottom),
            );
            path.line_to(point(body_right - radius, bottom));
            quarter_circle_to(
                &mut path,
                point(body_right - radius, bottom),
                point(body_right, bottom),
                point(body_right, bottom - radius),
            );
            path.line_to(point(body_right, top + curve));
            quarter_circle_to(
                &mut path,
                point(body_right, top + curve),
                point(body_right, top),
                point(right, top),
            );
            path.close();
            if let Ok(path) = path.build() {
                // Continue the tab strip through the shoulders, with the same
                // theme-aware contrast against the transcript canvas.
                window.paint_path(path, rgb(palette().document_tab_bar_bg));
            }
        },
    )
    .absolute()
    .top_0()
    .bottom_0()
    .left(-curve)
    .right(-curve)
}

fn quarter_circle_to(
    path: &mut PathBuilder,
    from: Point<Pixels>,
    corner: Point<Pixels>,
    to: Point<Pixels>,
) {
    // Same circular cubic used by folder tabs.
    const K: f32 = 0.552_284_8;
    path.cubic_bezier_to(to, from + (corner - from) * K, to + (corner - to) * K);
}
