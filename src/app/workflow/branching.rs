use super::*;
use crate::{
    branching,
    commands::{RequestIntent, TurnInput},
};

impl ConduitApp {
    pub(in crate::app) fn session_fork_pending(&self, id: u64) -> bool {
        self.session(id).is_some_and(branching::pending)
            || self
                .session_control
                .runs
                .get(&id)
                .is_some_and(|run| matches!(run.intent, RequestIntent::Fork(_)))
    }

    pub(in crate::app) fn create_native_fork(
        &mut self,
        source: u64,
        through: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<u64, String> {
        if !self.writable
            || self.closing
            || self.composer_save_pending()
            || self.session_busy(source)
            || !self.store.cli.enabled(Provider::Codex)
        {
            return Err(label("workflow.unavailable").into());
        }
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| w.sessions.iter().any(|s| s.id == source))
            .ok_or_else(|| label("workflow.missing").to_owned())?;
        let session = self
            .session(source)
            .ok_or_else(|| label("workflow.missing").to_owned())?;
        let id = self.next_id;
        let fork = branching::prepare(session, through, id, unix_now())?;
        let request = branching::request(&fork)?;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| label("workflow.limit").to_owned())?;
        self.store.workspaces[index].sessions.insert(0, fork);
        if let Err(error) = self.start_turn(
            index,
            Some(id),
            Provider::Codex,
            TurnInput {
                text: label("branching.creating").into(),
                intent: RequestIntent::Fork(request),
            },
            true,
            cx,
        ) {
            self.store.workspaces[index]
                .sessions
                .retain(|session| session.id != id);
            return Err(error);
        }
        self.switch(index, Some(id), window, cx);
        self.sidebar
            .invalidate_workspace(&self.store.workspaces[index].path);
        cx.notify();
        Ok(id)
    }

    pub(in crate::app) fn retry_native_fork(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let session = self
            .session(id)
            .ok_or_else(|| label("workflow.missing").to_owned())?;
        let request = branching::request(session)?;
        let index = self
            .store
            .workspaces
            .iter()
            .position(|w| w.sessions.iter().any(|s| s.id == id))
            .ok_or_else(|| label("workflow.missing").to_owned())?;
        self.start_turn(
            index,
            Some(id),
            Provider::Codex,
            TurnInput {
                text: label("branching.creating").into(),
                intent: RequestIntent::Fork(request),
            },
            true,
            cx,
        )?;
        cx.notify();
        Ok(())
    }
}

impl WorkflowPanel {
    pub(super) fn native_branch_controls(
        &self,
        writable: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(host) = self.host.upgrade() else {
            return div().into_any_element();
        };
        let app = host.read(cx);
        let source = self.source_session.and_then(|id| {
            app.store
                .workspaces
                .iter()
                .find(|w| w.path == self.workspace)?
                .sessions
                .iter()
                .find(|s| s.id == id)
        });
        let Some(source) = source else {
            return div().into_any_element();
        };
        let id = source.id;
        if branching::pending(source) {
            return v_flex()
                .gap_2()
                .child(label("branching.pending"))
                .child(
                    Button::new("native-fork-retry")
                        .small()
                        .label(label("branching.retry"))
                        .disabled(!writable || app.session_busy(id))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let result = this.host().and_then(|host| {
                                host.update(cx, |app, cx| app.retry_native_fork(id, cx))
                            });
                            this.result(result, cx);
                        })),
                )
                .into_any_element();
        }
        let through = self.through.read(cx).value().parse::<usize>().unwrap_or(0);
        let availability = if app.session_busy(id) {
            Err(label("branching.wait").to_owned())
        } else {
            branching::point_at(source, through).map(|_| ())
        };
        let points: Vec<_> = source
            .branching
            .points
            .iter()
            .rev()
            .filter(|point| branching::point_at(source, point.through).is_ok())
            .map(|point| point.through)
            .collect();
        let owner = cx.weak_entity();
        let mut panel = v_flex().gap_2().child(label("branching.hint")).child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    Button::new("native-fork-points")
                        .small()
                        .ghost()
                        .label(label("branching.points"))
                        .disabled(points.is_empty())
                        .dropdown_menu(move |mut menu, _, _| {
                            for through in &points {
                                let through = *through;
                                let owner = owner.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(conduit::i18n::format(
                                        "branching.point",
                                        &[("count", through.to_string())],
                                    ))
                                    .on_click(
                                        move |_, window, cx| {
                                            let _ = owner.update(cx, |this, cx| {
                                                this.through.update(cx, |input, cx| {
                                                    input.set_value(through.to_string(), window, cx)
                                                });
                                                cx.notify();
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                )
                .child(
                    Button::new("native-fork-create")
                        .small()
                        .label(label("branching.create"))
                        .disabled(!writable || availability.is_err())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let result = this.host().and_then(|host| {
                                host.update(cx, |app, cx| {
                                    app.create_native_fork(id, through, window, cx)
                                })
                            });
                            if result.is_ok() {
                                window.close_dialog(cx);
                            }
                            this.result(result, cx);
                        })),
                ),
        );
        if let Err(reason) = availability {
            panel = panel.child(div().text_size(ui_text_size(11.)).child(reason));
        }
        panel.into_any_element()
    }
}
