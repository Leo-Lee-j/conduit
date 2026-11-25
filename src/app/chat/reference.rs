use super::*;

/// Each column owns editor and reading state; both render the same components.
pub(in crate::app) struct ReferenceChatView {
    app: WeakEntity<ConduitApp>,
    session: u64,
    pub(in crate::app) input: Entity<TextareaState>,
    timeline: Entity<super::super::shell::ChatTimeline>,
    pub(in crate::app) source: super::super::shell::TimelineSource,
    pub(in crate::app) chat_focus: FocusHandle,
    _subscription: Subscription,
}

impl ReferenceChatView {
    pub(in crate::app) fn new(
        app: WeakEntity<ConduitApp>,
        session: u64,
        draft: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let scroll = gpui_kit::ListState::new(0, gpui_kit::ListAlignment::Bottom, px(240.));
        let source = super::super::shell::TimelineSource::session(session, scroll.clone());
        let timeline = cx.new(|_| {
            super::super::shell::ChatTimeline::for_session(app.clone(), scroll, source.clone())
        });
        let subscription = cx.observe(&app.upgrade().expect("app is alive"), |_, _, cx| {
            cx.notify()
        });
        Self {
            app,
            session,
            timeline,
            source,
            chat_focus: cx.focus_handle(),
            _subscription: subscription,
            input: cx.new(|cx| super::super::composer::composer_input_state(draft, window, cx)),
        }
    }

    pub(in crate::app) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.read(cx).focus_handle(cx).focus(window, cx);
    }
}

impl Render for ReferenceChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.app.upgrade() else {
            return div().into_any_element();
        };
        let loaded = app
            .read(cx)
            .session(self.session)
            .is_some_and(|session| session.transcript.loaded);
        if !loaded {
            app.update(cx, |app, cx| app.request_transcript(self.session, cx));
        }
        self.source.refresh(app.read(cx));
        let messages = app.update(cx, |app, cx| {
            app.sync_chat_timeline(&self.timeline, &self.source, cx);
            let selected = app.session(self.session).map(SessionSummary::from);
            app.render_chat_messages(
                selected.as_ref(),
                self.timeline.clone(),
                self.source.clone(),
                self.chat_focus.clone(),
                cx,
            )
        });
        let approval = app.update(cx, |app, cx| {
            app.render_chat_approval(self.session, window, cx)
        });
        let composer = app.update(cx, |app, cx| {
            app.composer_binding_for_session(self.session, self.input.clone())
                .map(|binding| app.render_composer_for(binding, window, cx))
        });
        v_flex()
            .id("reference-chat-view")
            .debug_selector(|| "reference-chat-view".into())
            .w_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(messages)
            .children(approval)
            .child(
                div()
                    .id("reference-chat-composer")
                    .debug_selector(|| "reference-chat-composer".into())
                    .flex_shrink_0()
                    .children(composer),
            )
            .into_any_element()
    }
}

impl ConduitApp {
    pub(in crate::app) fn chat_composer_binding(
        &self,
        session: u64,
    ) -> Option<super::super::composer::ComposerBinding> {
        let (workspace, _) = self
            .session_control
            .locate(&self.store.workspaces, session)?;
        self.live_composer_for_target(&super::super::composer::ComposerTarget {
            workspace: self.store.workspaces[workspace].path.clone(),
            session: Some(session),
        })
    }

    pub(in crate::app) fn remeasure_session_message(&self, session: u64, index: usize, cx: &App) {
        if self.selected_session == Some(session) {
            self.remeasure_message(index);
        } else if let Some(view) = self.reference_chat_view(session) {
            if let super::super::shell::TimelineSource::Session(state) = &view.read(cx).source {
                let state = state.borrow();
                state.rows.invalidate();
                if let Some(row) = state.rows.row(index) {
                    state.scroll.remeasure_items(row..row + 1);
                }
            }
        } else {
            self.remeasure_subagent_message(session, index);
        }
    }

    pub(in crate::app) fn session_message_group(
        &self,
        id: u64,
        index: usize,
        cx: &App,
    ) -> Option<rows::Group> {
        if self.selected_session == Some(id) {
            self.chat_rows.group(index)
        } else {
            self.reference_chat_view(id)
                .and_then(|view| view.read(cx).source.group(index))
        }
    }
}
