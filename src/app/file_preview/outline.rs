//! Cached Markdown headings and navigation within a file preview.
use super::*;
use gpui_kit::base::text::editor_host::{BlockNode, parse_markdown};

pub(super) const WIDTH: f32 = 224.;
const WIDE_VIEWPORT: f32 = 800.;
const ROW_HEIGHT: f32 = 28.;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Heading {
    level: u8,
    title: SharedString,
    source: Range<usize>,
}

#[derive(Default)]
pub(super) struct Outline {
    pub(super) headings: Arc<Vec<Heading>>,
    expanded: Option<bool>,
    scroll: UniformListScrollHandle,
    pub(super) pending: Rc<Cell<bool>>,
    target: Option<Range<usize>>,
    task: Option<Task<()>>,
}

impl Outline {
    pub(super) fn cancel_reveal(&mut self) {
        self.target = None;
        self.pending.set(false);
    }

    pub(super) fn navigation_active(&self) -> bool {
        self.target.is_some()
    }
}

fn headings(source: &str) -> Vec<Heading> {
    fn collect(blocks: &[BlockNode], out: &mut Vec<Heading>) {
        for block in blocks {
            match block {
                BlockNode::Heading {
                    level,
                    children,
                    span: Some(span),
                } => {
                    let text: String = children
                        .children()
                        .iter()
                        .map(|run| {
                            run.as_image()
                                .and_then(|image| image.alt.as_deref())
                                .unwrap_or_else(|| run.text())
                        })
                        .collect();
                    let title = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !title.is_empty() {
                        out.push(Heading {
                            level: *level,
                            title: title.into(),
                            source: span.start..span.end,
                        });
                    }
                }
                BlockNode::Root { children, .. }
                | BlockNode::Blockquote { children, .. }
                | BlockNode::List { children, .. }
                | BlockNode::ListItem { children, .. } => collect(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(blocks) = parse_markdown(source, Arc::default()) {
        collect(&blocks, &mut out);
    }
    out
}

pub(super) fn reveal_heading(scroll: &ScrollHandle, bounds: Bounds<Pixels>, window: &mut Window) {
    let y = (scroll.offset().y + scroll.bounds().top() + px(12.) - bounds.top())
        .clamp(-scroll.max_offset().y, px(0.));
    scroll.set_offset(point(px(0.), y));
    window.refresh();
}

impl FilePreview {
    pub(super) fn refresh_outline(&mut self, cx: &mut Context<Self>) {
        self.outline.task = None;
        self.outline.headings = Arc::default();
        self.outline.cancel_reveal();
        if self.markdown.is_none() {
            return;
        }
        let source = self.source.clone();
        let revision = self.revision;
        self.outline.task = Some(cx.spawn(async move |this, cx| {
            let headings = cx
                .background_executor()
                .spawn(async move { headings(&source) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.revision != revision {
                    return;
                }
                this.outline.headings = Arc::new(headings);
                this.outline.scroll = UniformListScrollHandle::new();
                cx.notify();
            });
        }));
    }

    pub(super) fn outline_wide(&self) -> bool {
        f32::from(self.viewport_size.get().width) >= WIDE_VIEWPORT
    }

    pub(super) fn outline_expanded(&self) -> bool {
        !self.outline.headings.is_empty()
            && self.outline.expanded.unwrap_or_else(|| self.outline_wide())
    }

    pub(super) fn reveal_outline(&self, cx: &mut Context<Self>) {
        if !self.outline.pending.get() {
            return;
        }
        if let Some(state) = &self.markdown
            && let Some(target) = &self.outline.target
            && let Some(rendered) = self
                .rendered
                .as_ref()
                .filter(|r| r.source() == self.source.as_ref())
            && let Some(range) = rendered.range_for_source(target.clone())
        {
            state.update(cx, |state, cx| {
                let _ = state.reveal_range(range, cx);
            });
        }
    }

    fn jump_to_heading(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(heading) = self.outline.headings.get(index) else {
            return;
        };
        self.outline.target = Some(heading.source.clone());
        self.pending.set(false);
        self.location_pending.set(false);
        self.outline.pending.set(true);
        if !self.outline_wide() {
            self.outline.expanded = Some(false);
        }
        self.focus.focus(window, cx);
        self.reveal_outline(cx);
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_outline(&self, expanded: bool, cx: &mut Context<Self>) -> AnyElement {
        let toggle = Button::new("file-outline-toggle")
            .debug_selector(|| "file-outline-toggle".into())
            .small()
            .compact()
            .icon(if expanded {
                FullIconName::X
            } else {
                FullIconName::List
            })
            .tooltip(conduit::i18n::text_static(if expanded {
                "file_preview.hide_outline"
            } else {
                "file_preview.show_outline"
            }))
            .accessibility_label(conduit::i18n::text_static(if expanded {
                "file_preview.hide_outline"
            } else {
                "file_preview.show_outline"
            }))
            .toggled(expanded)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.outline.expanded = Some(!expanded);
                this.focus.focus(window, cx);
                cx.stop_propagation();
                cx.notify();
            }));
        if !expanded {
            return div()
                .absolute()
                .top(px(12.))
                .right(px(16.))
                .child(toggle.label(conduit::i18n::text_static("file_preview.outline")))
                .into_any_element();
        }
        let owner = cx.entity();
        let min_level = self
            .outline
            .headings
            .iter()
            .map(|h| h.level)
            .min()
            .unwrap_or(1);
        let list = uniform_list(
            "file-outline-list",
            self.outline.headings.len(),
            move |range, _, cx| {
                owner.update(cx, |this, cx| {
                    range
                        .map(|index| {
                            let heading = &this.outline.headings[index];
                            let indent = f32::from(heading.level.saturating_sub(min_level)) * 12.;
                            let selector = format!("file-outline-heading-{index}");
                            Button::new(("file-outline-heading", index))
                                .debug_selector(move || selector.clone())
                                .ghost()
                                .w_full()
                                .h(px(ROW_HEIGHT))
                                .justify_start()
                                .pl(px(8. + indent))
                                .pr(px(8.))
                                .py_0()
                                .text_size(ui_text_size(12.))
                                .accessibility_label(heading.title.clone())
                                .tooltip(heading.title.clone())
                                .child(
                                    div()
                                        .w_full()
                                        .min_w_0()
                                        .truncate()
                                        .child(heading.title.clone()),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.jump_to_heading(index, window, cx);
                                }))
                        })
                        .collect::<Vec<_>>()
                })
            },
        )
        .debug_selector(|| "file-outline-list".into())
        .w_full()
        .h(px(self.outline.headings.len() as f32 * ROW_HEIGHT))
        .max_h(px(
            (f32::from(self.viewport_size.get().height) - 68.).max(0.)
        ))
        .min_h_0()
        .track_scroll(&self.outline.scroll);
        v_flex()
            .id("file-outline")
            .debug_selector(|| "file-outline".into())
            .occlude()
            .absolute()
            .top(px(12.))
            .right(px(16.))
            .w(px(WIDTH.min(
                (f32::from(self.viewport_size.get().width) - 32.).max(0.),
            )))
            .min_h_0()
            .p_1()
            .rounded(px(8.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .shadow_sm()
            .child(
                h_flex()
                    .w_full()
                    .h(px(32.))
                    .px_2()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_text_size(12.))
                            .font_semibold()
                            .child(conduit::i18n::text_static("file_preview.outline")),
                    )
                    .child(toggle.ghost()),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .min_h_0()
                    .child(list)
                    .scrollbar(&self.outline.scroll, ScrollbarAxis::Vertical),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::headings;

    #[test]
    fn outline_uses_rendered_heading_text_and_original_utf8_positions() {
        let source = "# 标题 **bold** [link](https://example.com) `code` &amp; ![图](image.png)\r\n\r\n## 重复\r\n\r\n## 重复\r\n";
        let entries = headings(source);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].title, "标题 bold link code & 图");
        assert_eq!(entries[1].title, "重复");
        assert_eq!(entries[2].title, "重复");
        assert_eq!(&source[entries[1].source.clone()], "## 重复");
        assert_eq!(&source[entries[2].source.clone()], "## 重复");
        assert!(entries[1].source.end < entries[2].source.start);
    }

    #[test]
    fn outline_respects_markdown_blocks_and_heading_levels() {
        let source = "# Real\n\n```md\n# Fenced\n```\n\n    # Indented\n\n<!--\n# Comment\n-->\n\nSetext\n------\n\n> ### Quote\n\n- #### List\n\n###### Deep\n\n#\n";
        let entries = headings(source);
        assert_eq!(
            entries
                .iter()
                .map(|h| (h.level, h.title.as_ref()))
                .collect::<Vec<_>>(),
            [
                (1, "Real"),
                (2, "Setext"),
                (3, "Quote"),
                (4, "List"),
                (6, "Deep")
            ]
        );
        for entry in entries {
            assert!(source.is_char_boundary(entry.source.start));
            assert!(source.is_char_boundary(entry.source.end));
        }
        assert!(headings("Plain text\n\n```\n# Code\n```").is_empty());
    }

    #[cfg(feature = "ui-tests")]
    #[gpui_kit::test]
    fn outline_navigation_survives_find_resize_and_content_replacement(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use super::super::*;
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let source = format!(
            "# 开始\n\n{}## 重复\n\n{}## 重复\n\n{}",
            "Paragraph 中文.\n\n".repeat(65),
            "Another paragraph.\n\n".repeat(65),
            "Ending paragraph.\n\n".repeat(25),
        );
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        struct PreviewHost(Entity<FilePreview>);
        impl Render for PreviewHost {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                v_flex().size_full().child(self.0.clone())
            }
        }
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = FilePreview::new(true, ScrollHandle::new(), window, cx);
                view.sync(
                    source.into(),
                    Arc::default(),
                    false,
                    view.typography.clone(),
                    cx,
                );
                view
            });
            *output.borrow_mut() = Some(view.clone());
            let host = cx.new(|_| PreviewHost(view));
            gpui_kit::component::Root::new(host, window, cx)
        });
        let view = slot.borrow().as_ref().unwrap().clone();
        fn draw(cx: &mut VisualTestContext) {
            for _ in 0..4 {
                cx.run_until_parked();
                cx.update(|window, cx| {
                    window.simulate_next_frame(cx);
                    let _ = window.draw(cx);
                });
            }
        }
        fn click(cx: &mut VisualTestContext, selector: &'static str) {
            let position = cx
                .debug_bounds(selector)
                .unwrap_or_else(|| {
                    panic!(
                        "{selector}: outline {:?}, list {:?}, viewport {:?}",
                        cx.debug_bounds("file-outline"),
                        cx.debug_bounds("file-outline-list"),
                        cx.debug_bounds("file-content-viewport")
                    )
                })
                .center();
            cx.simulate_mouse_move(position, None, Modifiers::default());
            cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
            draw(cx);
        }
        cx.simulate_resize(size(px(1100.), px(700.)));
        draw(cx);
        assert!(cx.debug_bounds("file-outline").is_some());
        cx.update(|window, cx| view.update(cx, |view, cx| view.open_find(window, cx)));
        draw(cx);
        cx.simulate_input("Paragraph");
        draw(cx);
        click(cx, "file-outline-heading-1");
        let first = cx.update(|_, cx| view.read(cx).scroll.offset().y);
        cx.executor().advance_clock(SEARCH_DELAY);
        draw(cx);
        cx.update(|_, cx| {
            let view = view.read(cx);
            assert!(!view.searching && !view.hits.is_empty());
            assert_eq!(
                view.scroll.offset().y,
                first,
                "late find results must not override a heading jump"
            );
        });
        click(cx, "file-outline-heading-2");
        cx.update(|_, cx| {
            let view = view.read(cx);
            assert!(first < -view.scroll.bounds().size.height);
            assert!(
                view.scroll.offset().y < first,
                "duplicate headings have distinct targets"
            );
            assert!(view.open, "outline navigation preserves the find bar");
        });
        let second = cx.update(|_, cx| view.read(cx).scroll.offset().y);
        cx.update(|window, cx| view.update(cx, |view, cx| view.navigate(false, window, cx)));
        draw(cx);
        cx.update(|_, cx| {
            assert!(
                view.read(cx).scroll.offset().y > second,
                "explicit find navigation takes precedence"
            )
        });
        click(cx, "file-outline-heading-0");
        cx.update(|_, cx| assert_eq!(view.read(cx).scroll.offset().y, px(0.)));
        cx.simulate_resize(size(px(600.), px(500.)));
        draw(cx);
        assert!(cx.debug_bounds("file-outline").is_none());
        click(cx, "file-outline-toggle");
        assert!(cx.debug_bounds("file-outline").is_some());
        click(cx, "file-outline-heading-2");
        assert!(
            cx.debug_bounds("file-outline").is_none(),
            "narrow navigation returns to reading"
        );
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                for source in [
                    "# Stale heading",
                    "# Latest heading",
                    "Plain text without headings",
                ] {
                    view.sync(
                        source.into(),
                        Arc::default(),
                        false,
                        view.typography.clone(),
                        cx,
                    );
                }
            })
        });
        draw(cx);
        cx.update(|_, cx| assert!(view.read(cx).outline.headings.is_empty()));
        assert!(cx.debug_bounds("file-outline-toggle").is_none());
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                let source: String = (0..80)
                    .map(|index| format!("## Section {index}\n\nSection body.\n\n"))
                    .collect();
                view.sync(
                    source.into(),
                    Arc::default(),
                    false,
                    view.typography.clone(),
                    cx,
                );
            })
        });
        draw(cx);
        click(cx, "file-outline-toggle");
        let before = cx.update(|_, cx| view.read(cx).scroll.offset());
        let position = cx.debug_bounds("file-outline-list").unwrap().center();
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.), px(-700.))),
            ..Default::default()
        });
        draw(cx);
        assert!(
            cx.debug_bounds("file-outline-heading-0").is_none(),
            "long outlines scroll"
        );
        cx.update(|_, cx| {
            assert_eq!(
                view.read(cx).scroll.offset(),
                before,
                "outline scrolling leaves the document in place"
            )
        });
        cx.update(|window, _| window.remove_window());
        slot.borrow_mut().take();
        drop(view);
        cx.run_until_parked();
    }
}
