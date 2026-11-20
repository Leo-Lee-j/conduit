//! Optional snapshot review. Normal file reads continue while a replay is paused.
use super::*;
use changes::{ChangeSet, Replay};

#[derive(Default)]
pub(super) struct State {
    pub latest: SharedString,
    pub changes: Option<Arc<ChangeSet>>,
    updated: bool,
    pub playback: Option<Playback>,
    task: Option<Task<()>>,
    selected: usize,
    located: bool,
    matched: Option<(u64, String)>,
    active_session: Option<u64>,
    following: Option<u64>,
    follow_paused: bool,
    visible: bool,
}
pub(super) struct Playback {
    plan: Replay,
    step: usize,
    total: usize,
    paused: bool,
    pub caret: usize,
    markdown: Option<Entity<TextViewState>>,
    highlights: Arc<Vec<(Range<usize>, &'static str)>>,
    scroll: Point<Pixels>,
}

pub(in crate::app) fn mark_color(deleted: bool) -> Hsla {
    let mut color = Hsla::from(rgb(if deleted {
        palette().error_text
    } else {
        palette().git_added_text
    }));
    color.a = 0.22;
    color
}

impl FilePreview {
    pub(in crate::app) fn set_changes(
        &mut self,
        changes: Option<Arc<ChangeSet>>,
        updated: bool,
        matched: Option<(u64, String)>,
        active_session: Option<u64>,
        cx: &mut Context<Self>,
    ) {
        let new_snapshot = match (&self.review.changes, &changes) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        let changed = new_snapshot
            || self.review.updated != updated
            || self.review.matched != matched
            || self.review.active_session != active_session;
        if !changed {
            return;
        }
        if new_snapshot {
            self.stop_replay(cx);
            self.review.selected = 0;
            self.review.located = false;
        }
        self.review.changes = changes;
        self.review.updated = updated;
        let newly_matched = matched.is_some() && self.review.matched != matched;
        self.review.matched = matched;
        self.review.active_session = active_session;
        if self.review.following.is_some() && self.review.following != active_session {
            self.review.follow_paused = true;
        }
        self.apply_highlights(cx);
        if (new_snapshot || newly_matched)
            && self.review.following == active_session
            && self.review.following.is_some()
            && !self.review.follow_paused
            && self.review.matched.as_ref().map(|m| m.0) == active_session
            && self.review.visible
        {
            self.reveal_change(false, cx);
        }
        cx.notify();
    }

    pub(in crate::app) fn set_review_visibility(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.review.visible == visible {
            return;
        }
        self.review.visible = visible;
        if !visible {
            self.pause_review(cx);
            self.stop_replay(cx);
        }
    }

    pub(in crate::app) fn visible_source(&self) -> SharedString {
        self.source.clone()
    }

    pub(in crate::app) fn pause_replay(&mut self, cx: &mut Context<Self>) {
        if let Some(playback) = &mut self.review.playback
            && !playback.paused
        {
            playback.paused = true;
            self.review.task = None;
            cx.notify();
        }
    }
    pub(in crate::app) fn pause_review(&mut self, cx: &mut Context<Self>) {
        self.pause_replay(cx);
        if self.review.following.is_some() && !self.review.follow_paused {
            self.review.follow_paused = true;
            cx.notify();
        }
    }

    pub(super) fn review_caret(&self) -> Option<usize> {
        if let Some(playback) = &self.review.playback {
            return Some(playback.caret);
        }
        self.review.matched.as_ref()?;
        let position = self
            .review
            .changes
            .as_ref()?
            .hunks
            .get(self.review.selected)?
            .new
            .start;
        if self.markdown.is_some() {
            self.rendered
                .as_ref()?
                .range_for_source(position..position)
                .map(|range| range.start)
        } else {
            Some(position)
        }
    }

    pub(super) fn review_marks(&self) -> Vec<(Range<usize>, bool)> {
        let Some(changes) = &self.review.changes else {
            return vec![];
        };
        if let Some(playback) = &self.review.playback {
            return vec![(
                if playback.step == 0 {
                    playback.plan.start..playback.plan.old_end
                } else {
                    playback.plan.start..playback.caret
                },
                playback.step == 0,
            )];
        }
        changes
            .hunks
            .iter()
            .filter_map(|hunk| {
                let range = if let Some(rendered) = &self.rendered
                    && self.markdown.is_some()
                {
                    rendered.range_for_source(hunk.new.clone())?
                } else if self.markdown.is_some() {
                    return None;
                } else {
                    hunk.new.clone()
                };
                Some((range, hunk.added == 0))
            })
            .collect()
    }

    fn replace_display(&mut self, text: SharedString, cx: &mut Context<Self>) {
        self.source = text;
        self.revision += 1;
        self.location_pending.set(false);
        if let Some(markdown) = &self.markdown {
            self.rendered = None;
            markdown.update(cx, |state, cx| state.set_text(&self.source, cx));
        }
        self.refresh_outline(cx);
        self.search(cx);
        cx.notify();
    }

    pub(in crate::app) fn start_replay(&mut self, cx: &mut Context<Self>) {
        let Some(changes) = self.review.changes.clone() else {
            return;
        };
        self.stop_replay(cx);
        self.review.follow_paused = true;
        let plan = changes.replay();
        let caret = plan.start;
        self.review.playback = Some(Playback {
            plan,
            step: 0,
            total: if changes.before.len() + changes.after.len() > 16 * 1024 {
                8
            } else {
                48
            },
            paused: cx.reduce_motion(),
            caret,
            markdown: self.markdown.take(),
            highlights: self.highlights.clone(),
            scroll: self.scroll.offset(),
        });
        self.highlights = Arc::default();
        self.replace_display(changes.before.to_string().into(), cx);
        let line = changes.before[..caret]
            .bytes()
            .filter(|b| *b == b'\n')
            .count()
            + 1;
        self.go_to(SourcePosition { line, column: 1 }, cx);
        if cx.reduce_motion() {
            if let Some(p) = &mut self.review.playback {
                p.step = p.total;
            }
            self.update_replay_frame(cx);
        } else {
            self.schedule_replay(cx);
        }
    }

    fn schedule_replay(&mut self, cx: &mut Context<Self>) {
        self.review.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(30))
                    .await;
                let keep_going = this
                    .update(cx, |this, cx| {
                        let Some(p) = &mut this.review.playback else {
                            return false;
                        };
                        if p.paused || !this.review.visible {
                            return false;
                        }
                        p.step = (p.step + 1).min(p.total);
                        p.paused = p.step == p.total;
                        this.update_replay_frame(cx);
                        this.review.playback.as_ref().is_some_and(|p| !p.paused)
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        }));
    }

    fn update_replay_frame(&mut self, cx: &mut Context<Self>) {
        let Some(changes) = &self.review.changes else {
            return;
        };
        let Some(p) = &mut self.review.playback else {
            return;
        };
        let (text, caret) = p.plan.frame(changes, p.step, p.total);
        p.caret = caret;
        self.replace_display(text.into(), cx);
    }

    pub(in crate::app) fn stop_replay(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.review.playback.take() else {
            return;
        };
        self.review.task = None;
        self.markdown = p.markdown;
        self.highlights = p.highlights;
        self.replace_display(self.review.latest.clone(), cx);
        self.scroll.set_offset(p.scroll);
        self.apply_highlights(cx);
    }

    fn reveal_change(&mut self, next: bool, cx: &mut Context<Self>) {
        let Some(changes) = &self.review.changes else {
            return;
        };
        if next && self.review.located {
            self.review.selected = (self.review.selected + 1) % changes.hunks.len();
        }
        self.review.located = true;
        let line = changes.hunks[self.review.selected].line;
        self.go_to(SourcePosition { line, column: 1 }, cx);
    }

    pub(super) fn review_bar(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if cx.reduce_motion() && self.review.playback.as_ref().is_some_and(|p| !p.paused) {
            self.pause_replay(cx);
            if let Some(p) = &mut self.review.playback {
                p.step = p.total;
            }
            self.update_replay_frame(cx);
        }
        if !self.review.updated {
            return None;
        }
        let mut bar = v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(rgb(palette().border))
            .text_size(ui_text_size(11.));
        let label = if self.review.playback.is_some() {
            "files.changes.replaying"
        } else if self.review.matched.is_some() {
            "files.changes.matched"
        } else {
            "files.changes.updated"
        };
        let mut summary = conduit::i18n::text_static(label).to_string();
        if self.review.playback.is_none()
            && let Some((_, title)) = &self.review.matched
        {
            summary.push_str(&format!(" \u{b7} {title}"));
        }
        if let Some(changes) = &self.review.changes {
            summary.push_str(&format!("  +{} / −{}", changes.added, changes.removed));
        }
        if self.review.playback.is_none()
            && let Some(changes) = &self.review.changes
        {
            let hunk = &changes.hunks[self.review.selected];
            summary.push_str(&format!(
                "  L{} (+{} / -{})",
                hunk.line, hunk.added, hunk.removed
            ));
        }
        bar = bar.child(div().min_w_0().truncate().child(summary));
        if let Some(p) = self.review.playback.as_ref() {
            let paused = p.paused;
            let complete = p.step == p.total;
            bar = bar.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("file-replay-pause")
                            .ghost()
                            .xsmall()
                            .label(conduit::i18n::text_static(if complete {
                                "files.changes.restart"
                            } else if paused {
                                "files.changes.resume"
                            } else {
                                "files.changes.pause"
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if complete {
                                    this.start_replay(cx);
                                } else if paused {
                                    this.open = false;
                                    this.apply_highlights(cx);
                                    if let Some(p) = &mut this.review.playback {
                                        p.paused = false;
                                    }
                                    this.schedule_replay(cx);
                                } else {
                                    this.pause_replay(cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("file-replay-return")
                            .ghost()
                            .xsmall()
                            .label(conduit::i18n::text_static("files.changes.return_current"))
                            .on_click(cx.listener(|this, _, _, cx| this.stop_replay(cx))),
                    ),
            );
        } else if self.review.changes.is_some() {
            let following = self.review.following == self.review.active_session
                && self.review.following.is_some();
            let paused = self.review.follow_paused;
            bar = bar.child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("file-change-next")
                            .ghost()
                            .xsmall()
                            .label(conduit::i18n::text_static("files.changes.next"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pause_review(cx);
                                this.reveal_change(true, cx);
                            })),
                    )
                    .child(
                        Button::new("file-change-replay")
                            .ghost()
                            .xsmall()
                            .label(conduit::i18n::text_static("files.changes.replay"))
                            .on_click(cx.listener(|this, _, _, cx| this.start_replay(cx))),
                    )
                    .child(
                        Button::new("file-change-follow")
                            .ghost()
                            .xsmall()
                            .label(conduit::i18n::text_static(if following && !paused {
                                "files.changes.stop_follow"
                            } else if following {
                                "files.changes.resume_follow"
                            } else {
                                "files.changes.follow"
                            }))
                            .disabled(self.review.active_session.is_none())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if following && !paused {
                                    this.review.following = None;
                                } else {
                                    this.review.following = this.review.active_session;
                                    this.review.follow_paused = false;
                                }
                                cx.notify();
                            })),
                    ),
            );
        } else {
            bar = bar.child(
                div()
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static("files.changes.summary_only")),
            );
        }
        Some(bar.into_any_element())
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ChangeSet, FilePreview};
    use gpui_kit::{AppContext, ScrollHandle, TestAppContext};
    use std::sync::Arc;

    #[gpui_kit::test]
    fn content_review_markdown_search_copy_and_reduced_motion_share_visible_snapshot(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (_, cx, _) = crate::app::queue::tests::fixture(cx, "content-review-markdown");
        let preview = cx.update(|window, cx| {
            cx.new(|cx| FilePreview::new(true, ScrollHandle::new(), window, cx))
        });
        let changes =
            Arc::new(ChangeSet::between("# Before\nold text\n", "# After\nnew text\n").unwrap());
        cx.update(|window, cx| {
            preview.update(cx, |preview, cx| {
                let typography = preview.typography.clone();
                preview.sync(
                    changes.after.to_string().into(),
                    Arc::default(),
                    false,
                    typography,
                    cx,
                );
                preview.set_changes(
                    Some(changes.clone()),
                    true,
                    Some((1, "tool".into())),
                    Some(1),
                    cx,
                );
                preview.set_review_visibility(true, cx);
                preview.start_replay(cx);
                preview.open_find(window, cx);
                assert!(preview.review.playback.as_ref().unwrap().paused);
                assert!(preview.open);
                assert!(preview.markdown.is_none());
                assert_eq!(preview.visible_source(), changes.before.as_ref());
                let p = preview.review.playback.as_mut().unwrap();
                p.step = p.total;
                preview.update_replay_frame(cx);
                assert_eq!(preview.visible_source(), changes.after.as_ref());
                preview.stop_replay(cx);
                assert!(preview.markdown.is_some());
                assert_eq!(preview.visible_source(), changes.after.as_ref());
                preview.review.following = Some(1);
                preview.review.follow_paused = false;
                preview.set_changes(
                    Some(changes.clone()),
                    true,
                    Some((1, "tool".into())),
                    Some(2),
                    cx,
                );
                assert!(preview.review.follow_paused);
                cx.set_reduce_motion(true);
                preview.start_replay(cx);
                assert!(preview.review.playback.as_ref().unwrap().paused);
                assert_eq!(preview.visible_source(), changes.after.as_ref());
                preview.set_review_visibility(false, cx);
                assert!(preview.review.playback.is_none());
                assert!(preview.markdown.is_some());
            })
        });
    }
}
