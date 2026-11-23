//! A transcript's identity and reading state are independent of its column.
use super::*;
use std::{cell::RefCell, rc::Rc};

pub(in crate::app) struct SessionTimeline {
    pub session: u64,
    pub rows: chat::rows::Rows,
    pub scroll: ListState,
    pub at_bottom: bool,
    pub outline: MessageOutlineCache,
    pub pending_message_jump: Option<usize>,
}

#[derive(Clone, Default)]
pub(in crate::app) enum TimelineSource {
    #[default]
    Main,
    Session(Rc<RefCell<SessionTimeline>>),
}

impl TimelineSource {
    pub(in crate::app) fn session(session: u64, scroll: ListState) -> Self {
        Self::Session(Rc::new(RefCell::new(SessionTimeline {
            session,
            scroll,
            rows: Default::default(),
            at_bottom: true,
            outline: Default::default(),
            pending_message_jump: None,
        })))
    }

    pub(in crate::app) fn id(&self, app: &ConduitApp) -> Option<u64> {
        match self {
            Self::Main => app.selected_session,
            Self::Session(state) => Some(state.borrow().session),
        }
    }

    pub(super) fn context(&self, app: &ConduitApp) -> (Option<u64>, u64) {
        match self {
            Self::Main => (app.selected_session, app.chat_rows.mapping_revision),
            Self::Session(state) => {
                let state = state.borrow();
                (Some(state.session), state.rows.mapping_revision)
            }
        }
    }

    pub(super) fn navigating(&self, app: &ConduitApp) -> bool {
        match self {
            Self::Main => app.pending_message_jump.is_some(),
            Self::Session(state) => state.borrow().pending_message_jump.is_some(),
        }
    }

    pub(in crate::app) fn following(&self, app: &ConduitApp) -> bool {
        match self {
            Self::Main => app.at_bottom,
            Self::Session(state) => state.borrow().at_bottom,
        }
    }

    pub(super) fn message(&self, app: &ConduitApp, row: usize) -> usize {
        match self {
            Self::Main => app.chat_rows.message(row),
            Self::Session(state) => state.borrow().rows.message(row),
        }
    }

    pub(in crate::app) fn group(&self, index: usize) -> Option<chat::rows::Group> {
        match self {
            Self::Main => None,
            Self::Session(state) => state.borrow().rows.group(index),
        }
    }

    pub(in crate::app) fn refresh(&self, app: &ConduitApp) {
        let Self::Session(state) = self else { return };
        let mut state = state.borrow_mut();
        let top = state.scroll.logical_scroll_top();
        let anchor = state.rows.rows.get(top.item_ix).map(|row| row.key.clone());
        let session = app.session(state.session);
        let revision = state.rows.revision.get();
        let old = state.rows.refresh(session, &app.tool_group_expansion);
        if let Some(old) = old
            && old != state.rows.rows
        {
            let rows = &state.rows.rows;
            let prefix = old.iter().zip(rows).take_while(|(a, b)| a == b).count();
            let suffix = old[prefix..]
                .iter()
                .rev()
                .zip(rows[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            state
                .scroll
                .splice(prefix..old.len() - suffix, rows.len() - prefix - suffix);
            if !state.at_bottom
                && let Some(anchor) = anchor
                && let Some(row) = rows.iter().position(|row| row.key == anchor)
            {
                state.scroll.scroll_to(ListOffset {
                    item_ix: row,
                    offset_in_item: top.offset_in_item,
                });
            }
        }
        // Content changes can affect heights without rebuilding the row map.
        // Unrelated renders must preserve measured rows and deferred wheel anchors.
        if state.rows.revision.get() != revision {
            state.scroll.remeasure();
        }
    }

    pub(in crate::app) fn jump_to_end(&self, app: &mut ConduitApp, cx: &mut Context<ConduitApp>) {
        match self {
            Self::Main => {
                app.scroll_to_end();
                app.at_bottom = true;
                app.message_focus = None;
                app.pending_message_jump = None;
            }
            Self::Session(state) => {
                let mut state = state.borrow_mut();
                state.at_bottom = true;
                state.pending_message_jump = None;
                state.scroll.scroll_to_end();
            }
        }
        cx.notify();
    }

    pub(super) fn scrolled(
        &self,
        scroll: &ListState,
        at_bottom: bool,
        app: &mut ConduitApp,
        cx: &mut Context<ConduitApp>,
    ) {
        match self {
            Self::Main => {
                let navigating = app.pending_message_jump.take().is_some();
                let outline = app
                    .outline
                    .active_for_item(app.chat_rows.message(scroll.logical_scroll_top().item_ix));
                if navigating || app.at_bottom != at_bottom || app.outline.active != outline {
                    app.at_bottom = at_bottom;
                    cx.notify();
                }
            }
            Self::Session(state) => {
                let mut state = state.borrow_mut();
                let navigating = state.pending_message_jump.take().is_some();
                if navigating || state.at_bottom != at_bottom {
                    state.at_bottom = at_bottom;
                    cx.notify();
                }
            }
        }
        self.refresh_active(app, cx);
    }

    pub(super) fn render(
        &self,
        app: &mut ConduitApp,
        row: usize,
        window: &mut Window,
        cx: &mut Context<ConduitApp>,
    ) -> (
        AnyElement,
        Option<Entity<gpui_kit::base::text::TextViewState>>,
    ) {
        match self {
            Self::Main => app.render_message(row, window, cx),
            Self::Session(state) => {
                let (id, index, group) = {
                    let state = state.borrow();
                    let index = state.rows.message(row);
                    (state.session, index, state.rows.group(index))
                };
                app.render_session_message(id, index, group, window, cx)
            }
        }
    }
}
