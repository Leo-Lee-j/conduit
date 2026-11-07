//! Inputs read by chat row rendering, excluding the composer and plugin panes.
//! Message bodies stay in the store: Rows tracks their immutable identities.
use super::*;

#[derive(PartialEq)]
pub(super) struct State {
    session: Option<(u64, Provider, u64, Option<String>, bool)>,
    workspace: Option<PathBuf>,
    revision: u64,
    typography: ChatTypography,
    palette: theme::Palette,
    full_width: bool,
    monitor: bool,
    writable: bool,
    closing: bool,
    busy: bool,
    live: bool,
    catalog: Option<(Option<String>, Vec<crate::models::ModelOption>)>,
    copied: Option<SharedString>,
    highlighted: Option<(u64, String)>,
    expanded: [HashSet<(u64, String)>; 3],
    scroll: Option<(usize, Pixels)>,
}

impl State {
    fn capture(app: &ConduitApp) -> Self {
        let session = app.selected_session.and_then(|id| app.session(id));
        let top = app.scroll.logical_scroll_top();
        Self {
            session: session.map(|s| {
                (
                    s.id,
                    s.provider,
                    s.created_at,
                    s.model.clone(),
                    s.interrupted,
                )
            }),
            workspace: session
                .and_then(|s| {
                    app.store
                        .workspaces
                        .iter()
                        .find(|w| w.sessions.iter().any(|candidate| candidate.id == s.id))
                })
                .map(|w| w.path.clone()),
            revision: app.chat_rows.revision.get(),
            typography: app.chat_typography(),
            palette: palette(),
            full_width: app.store.ui.full_width_content,
            monitor: app.store.ui.performance_monitor,
            writable: app.writable,
            closing: app.closing,
            busy: session.is_some_and(|s| app.session_busy(s.id)),
            live: session.is_some_and(|s| app.session_control.runs.contains_key(&s.id)),
            catalog: session
                .and_then(|s| app.catalogs.get(&s.provider))
                .map(|c| (c.current.clone(), c.models.clone())),
            copied: app.copied_action.clone(),
            highlighted: app
                .message_focus
                .as_ref()
                .map(|(id, key, _)| (*id, key.clone())),
            expanded: [
                app.expanded_tools.clone(),
                app.expanded_reasoning.clone(),
                app.expanded_errors.clone(),
            ],
            scroll: (!app.at_bottom || app.pending_message_jump.is_some())
                .then_some((top.item_ix, top.offset_in_item)),
        }
    }
}

impl ConduitApp {
    pub(super) fn sync_timeline(&self, cx: &mut Context<Self>) {
        let state = State::capture(self);
        // Keep the existing refresh behavior during live runs. Completed transcripts
        // reuse their layout/paint until a render input changes. GPUI additionally
        // invalidates cached views on bounds, inherited text style, and refresh.
        let live = state.live;
        self.timeline.update(cx, |timeline, cx| {
            if live || timeline.render_state.as_ref() != Some(&state) {
                timeline.render_state = Some(state);
                cx.notify();
            }
        });
    }
}
