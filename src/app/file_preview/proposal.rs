//! An explicitly selected, ephemeral source view for streamed tool arguments.
use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app) struct Snapshot {
    pub session: u64,
    pub key: String,
    pub text: SharedString,
    pub range: Range<usize>,
    pub complete: bool,
}
#[derive(Default)]
pub(super) struct State {
    snapshot: Option<Snapshot>,
    display: Option<Display>,
}
struct Display {
    markdown: Option<Entity<TextViewState>>,
    highlights: Arc<Vec<(Range<usize>, &'static str)>>,
    scroll: Point<Pixels>,
    paused: bool,
    range: Range<usize>,
}
impl FilePreview {
    pub(in crate::app) fn set_proposal(
        &mut self,
        snapshot: Option<Snapshot>,
        cx: &mut Context<Self>,
    ) {
        if self.proposal.snapshot == snapshot {
            return;
        }
        let same = self
            .proposal
            .snapshot
            .as_ref()
            .zip(snapshot.as_ref())
            .is_some_and(|(a, b)| a.session == b.session && a.key == b.key);
        if !same {
            self.stop_proposal(cx);
        }
        self.proposal.snapshot = snapshot;
        self.update_proposal_display(cx);
        cx.notify();
    }
    fn update_proposal_display(&mut self, cx: &mut Context<Self>) {
        if let (Some(snapshot), Some(display)) =
            (&self.proposal.snapshot, &mut self.proposal.display)
            && !display.paused
        {
            display.range = snapshot.range.clone();
            let text = snapshot.text.clone();
            self.replace_display(text, cx);
        }
    }
    pub(in crate::app) fn has_proposal(&self) -> bool {
        self.proposal.snapshot.is_some()
    }
    pub(super) fn retain_proposal_session(&mut self, session: Option<u64>, cx: &mut Context<Self>) {
        if self
            .proposal
            .snapshot
            .as_ref()
            .is_some_and(|p| Some(p.session) != session)
        {
            self.set_proposal(None, cx);
        }
    }
    pub(super) fn proposal_active(&self) -> bool {
        self.proposal.display.is_some()
    }
    pub(super) fn proposal_range(&self) -> Option<Range<usize>> {
        self.proposal.display.as_ref().map(|p| p.range.clone())
    }
    pub(super) fn pause_proposal(&mut self, cx: &mut Context<Self>) {
        if let Some(display) = &mut self.proposal.display
            && !display.paused
        {
            display.paused = true;
            cx.notify();
        }
    }
    pub(in crate::app) fn start_proposal(&mut self, cx: &mut Context<Self>) {
        if self.proposal.snapshot.is_none() || self.proposal_active() || !self.review.visible {
            return;
        }
        self.stop_replay(cx);
        self.review.follow_paused = true;
        self.proposal.display = Some(Display {
            markdown: self.markdown.take(),
            highlights: self.highlights.clone(),
            scroll: self.scroll.offset(),
            paused: false,
            range: 0..0,
        });
        self.rendered = None;
        self.highlights = Arc::default();
        self.update_proposal_display(cx);
        // No focus transfer or automatic scrolling while arguments arrive.
    }
    pub(super) fn stop_proposal(&mut self, cx: &mut Context<Self>) {
        if let Some(display) = self.proposal.display.take() {
            self.markdown = display.markdown;
            self.highlights = display.highlights;
            self.replace_display(self.review.latest.clone(), cx);
            self.scroll.set_offset(display.scroll);
            self.apply_highlights(cx);
        }
    }
    pub(super) fn proposal_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let snapshot = self.proposal.snapshot.as_ref()?;
        let active = self.proposal_active();
        let paused = self.proposal.display.as_ref().is_some_and(|d| d.paused);
        let label = if paused {
            "files.proposal.paused"
        } else if snapshot.complete {
            "files.proposal.ready"
        } else {
            "files.proposal.generating"
        };
        let mut buttons = h_flex().gap_1();
        if active {
            buttons = buttons
                .child(
                    Button::new("file-proposal-pause")
                        .debug_selector(|| "file-proposal-pause".into())
                        .ghost()
                        .xsmall()
                        .label(conduit::i18n::text_static(if paused {
                            "files.changes.resume"
                        } else {
                            "files.changes.pause"
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if paused {
                                if !this.close_find_for_review(window, cx) {
                                    return;
                                }
                                if let Some(display) = &mut this.proposal.display {
                                    display.paused = false;
                                }
                                this.update_proposal_display(cx);
                            } else {
                                this.pause_proposal(cx);
                            }
                        })),
                )
                .child(
                    Button::new("file-proposal-return")
                        .debug_selector(|| "file-proposal-return".into())
                        .ghost()
                        .xsmall()
                        .label(conduit::i18n::text_static("files.changes.return_current"))
                        .on_click(cx.listener(|this, _, _, cx| this.stop_proposal(cx))),
                );
        } else {
            buttons = buttons.child(
                Button::new("file-proposal-open")
                    .debug_selector(|| "file-proposal-open".into())
                    .ghost()
                    .xsmall()
                    .label(conduit::i18n::text_static("files.proposal.open"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.close_find_for_review(window, cx) {
                            this.start_proposal(cx);
                        }
                    })),
            );
        }
        Some(
            v_flex()
                .id("file-proposal-bar")
                .debug_selector(|| "file-proposal-bar".into())
                .w_full()
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(rgb(palette().border))
                .text_size(ui_text_size(11.))
                .child(conduit::i18n::text_static(label))
                .child(buttons)
                .into_any_element(),
        )
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{FilePreview, Snapshot};
    use gpui_kit::{AppContext, ScrollHandle, TestAppContext};
    use std::sync::Arc;
    #[gpui_kit::test]
    fn content_proposal_pointer_controls_survive_press_repaint_and_find(cx: &mut TestAppContext) {
        use super::super::*;
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        struct Host(Entity<FilePreview>, Entity<InputState>);
        impl Render for Host {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                v_flex()
                    .size_full()
                    .child(Input::new(&self.1))
                    .child(self.0.clone())
            }
        }
        let slot = Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                let mut view = FilePreview::new(true, ScrollHandle::new(), window, cx);
                view.sync(
                    "# disk\nold\n".into(),
                    Arc::default(),
                    false,
                    view.typography.clone(),
                    cx,
                );
                view.set_review_visibility(true, cx);
                view.set_changes(None, false, None, Some(1), cx);
                view.set_proposal(
                    Some(Snapshot {
                        session: 1,
                        key: "tool".into(),
                        text: "# disk\nnew\n".into(),
                        range: 7..10,
                        complete: false,
                    }),
                    cx,
                );
                view
            });
            let composer = cx.new(|cx| InputState::new(window, cx));
            *output.borrow_mut() = Some((view.clone(), composer.clone()));
            let host = cx.new(|_| Host(view, composer));
            gpui_kit::component::Root::new(host, window, cx)
        });
        let (view, composer) = slot.borrow().as_ref().unwrap().clone();
        fn draw(cx: &mut VisualTestContext) {
            for _ in 0..3 {
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
                .unwrap_or_else(|| panic!("missing {selector}"))
                .center();
            cx.simulate_mouse_move(position, None, Modifiers::default());
            cx.simulate_mouse_down(position, MouseButton::Left, Modifiers::default());
            // Real pointer presses can span frames. A parent selection handler
            // must not change Pause into Resume before mouse-up invokes it.
            draw(cx);
            cx.simulate_mouse_up(position, MouseButton::Left, Modifiers::default());
            draw(cx);
        }
        cx.simulate_resize(size(px(900.), px(600.)));
        draw(cx);
        click(cx, "file-proposal-open");
        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                assert!(view.proposal_active());
                assert!(!view.proposal.display.as_ref().unwrap().paused);
            })
        });
        click(cx, "file-proposal-pause");
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                assert!(
                    view.proposal.display.as_ref().unwrap().paused,
                    "Pause must remain paused after mouse-up"
                );
                let mut snapshot = view.proposal.snapshot.clone().unwrap();
                snapshot.text = "# disk\nnewer\n".into();
                snapshot.range = 7..12;
                view.set_proposal(Some(snapshot), cx);
                assert_eq!(view.visible_source(), "# disk\nnew\n");
            })
        });
        draw(cx);
        click(cx, "file-proposal-pause");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(!view.proposal.display.as_ref().unwrap().paused);
                assert_eq!(view.visible_source(), "# disk\nnewer\n");
                view.open_find(window, cx);
            })
        });
        draw(cx);
        cx.simulate_input("newer");
        draw(cx);
        click(cx, "file-proposal-pause");
        cx.update(|window, cx| {
            view.update(cx, |view, _| {
                assert!(
                    !view.open,
                    "Resume closes find so incoming frames cannot reveal search hits"
                );
                assert!(
                    view.focus.is_focused(window),
                    "Hidden find input must not retain focus"
                );
                assert!(!view.proposal.display.as_ref().unwrap().paused);
            })
        });
        cx.update(|window, cx| view.update(cx, |view, cx| view.open_find(window, cx)));
        draw(cx);
        cx.update(|window, cx| window.focus_next(cx));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(!view.input.read(cx).focus_handle(cx).is_focused(window));
                assert!(view.focus.contains_focused(window, cx));
                view.close_find(window, cx);
                assert!(
                    view.focus.is_focused(window),
                    "Closing find also retires focused find-bar buttons"
                );
            })
        });
        draw(cx);
        click(cx, "file-content-viewport");
        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                assert!(
                    view.proposal.display.as_ref().unwrap().paused,
                    "Reading the content still pauses updates"
                );
            })
        });
        click(cx, "file-proposal-return");
        cx.update(|_, cx| {
            view.update(cx, |view, _| {
                assert!(!view.proposal_active());
                assert!(view.markdown.is_some());
                assert_eq!(view.visible_source(), "# disk\nold\n");
            })
        });
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.set_proposal(None, cx);
                let changes = Arc::new(
                    changes::ChangeSet::between("# disk\nbefore\n", "# disk\nold\n").unwrap(),
                );
                view.set_changes(Some(changes), true, Some((1, "saved".into())), Some(1), cx);
            })
        });
        draw(cx);
        click(cx, "file-change-replay");
        click(cx, "file-replay-pause");
        let paused_source = cx.update(|_, cx| view.read(cx).visible_source());
        cx.executor().advance_clock(Duration::from_millis(350));
        draw(cx);
        cx.executor().advance_clock(Duration::from_millis(35));
        draw(cx);
        cx.update(|_, cx| assert_eq!(view.read(cx).visible_source(), paused_source));
        cx.update(|window, cx| view.update(cx, |view, cx| view.open_find(window, cx)));
        draw(cx);
        click(cx, "file-replay-pause");
        cx.update(|window, cx| {
            view.update(cx, |view, _| {
                assert!(!view.open);
                assert!(view.focus.is_focused(window));
                assert!(view.review.playback.is_some());
            })
        });
        cx.executor().advance_clock(Duration::from_millis(350));
        draw(cx);
        cx.executor().advance_clock(Duration::from_millis(35));
        draw(cx);
        cx.update(|_, cx| assert_ne!(view.read(cx).visible_source(), paused_source));
        click(cx, "file-replay-return");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.set_proposal(
                    Some(Snapshot {
                        session: 1,
                        key: "next".into(),
                        text: "# next\n".into(),
                        range: 0..7,
                        complete: false,
                    }),
                    cx,
                );
                view.open_find(window, cx);
            });
            composer.update(cx, |input, cx| input.focus_handle(cx).focus(window, cx));
        });
        draw(cx);
        click(cx, "file-proposal-open");
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert!(!view.open);
                assert!(view.proposal_active());
                assert!(
                    composer.read(cx).focus_handle(cx).is_focused(window),
                    "Closing find must preserve focus outside the preview"
                );
            })
        });
    }

    #[gpui_kit::test]
    fn content_proposal_visible_source_pause_find_restore_and_lifecycle(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (_, cx, _) = crate::app::queue::tests::fixture(cx, "content-proposal-view");
        let preview = cx.update(|window, cx| {
            cx.new(|cx| FilePreview::new(true, ScrollHandle::new(), window, cx))
        });
        cx.update(|window, cx| {
            preview.update(cx, |preview, cx| {
                let typography = preview.typography.clone();
                preview.sync(
                    "# disk\nold\n".into(),
                    Arc::default(),
                    false,
                    typography.clone(),
                    cx,
                );
                preview.set_review_visibility(true, cx);
                preview.set_changes(None, false, None, Some(1), cx);
                let mut snapshot = Snapshot {
                    session: 1,
                    key: "tool".into(),
                    text: "# disk\nnew\n".into(),
                    range: 7..10,
                    complete: false,
                };
                preview.set_proposal(Some(snapshot.clone()), cx);
                assert_eq!(preview.visible_source(), "# disk\nold\n");
                preview.start_proposal(cx);
                assert!(preview.markdown.is_none());
                assert_eq!(preview.visible_source(), snapshot.text);
                assert_eq!(preview.review_caret(), Some(10));
                // Find/selection/copy share this source and freeze the current frame.
                preview.open_find(window, cx);
                assert!(preview.proposal.display.as_ref().unwrap().paused);
                snapshot.text = "# disk\nnewer\n".into();
                snapshot.range = 7..12;
                preview.set_proposal(Some(snapshot.clone()), cx);
                assert_eq!(preview.visible_source(), "# disk\nnew\n");
                assert_eq!(preview.review_caret(), Some(10));
                assert_eq!(preview.review.latest, "# disk\nold\n");
                preview.proposal.display.as_mut().unwrap().paused = false;
                preview.update_proposal_display(cx);
                assert_eq!(preview.visible_source(), snapshot.text);
                preview.stop_proposal(cx);
                assert!(preview.markdown.is_some());
                assert_eq!(preview.visible_source(), "# disk\nold\n");
                preview.start_proposal(cx);
                preview.sync("# external\n".into(), Arc::default(), false, typography, cx);
                assert!(!preview.proposal_active());
                assert!(preview.markdown.is_some());
                assert_eq!(preview.visible_source(), "# external\n");
                preview.set_proposal(Some(snapshot.clone()), cx);
                preview.start_proposal(cx);
                preview.set_review_visibility(false, cx);
                assert!(preview.proposal.snapshot.is_none());
                assert!(preview.markdown.is_some());
                preview.set_review_visibility(true, cx);
                preview.set_proposal(Some(snapshot), cx);
                preview.start_proposal(cx);
                preview.set_changes(None, false, None, Some(2), cx);
                assert!(preview.proposal.snapshot.is_none());
                assert_eq!(preview.visible_source(), "# external\n");
            })
        });
    }
}
