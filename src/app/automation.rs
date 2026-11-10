//! Durable automation claims feed the existing session send barrier.
use super::*;
use crate::automation::{Status, Trigger};
mod panel;

pub(super) struct AutomationUi {
    recovered: bool,
    last_poll: Instant,
    pending: Vec<(u64, crate::persistence::SaveReceipt)>,
}
impl Default for AutomationUi {
    fn default() -> Self {
        Self {
            recovered: false,
            last_poll: Instant::now(),
            pending: vec![],
        }
    }
}

fn label(key: &'static str) -> &'static str {
    conduit::i18n::text_static(key)
}

impl ConduitApp {
    fn recover_automations(&mut self) {
        if !self.automation.recovered {
            self.automation.recovered = true;
            if self.store.scheduler.recover() {
                self.persist();
            }
        }
    }

    pub(super) fn poll_automations(&mut self, cx: &mut Context<Self>) {
        if !self.writable || self.closing || self.startup.is_some() {
            return;
        }
        self.recover_automations();
        for (id, receipt) in std::mem::take(&mut self.automation.pending) {
            match receipt.poll() {
                None => self.automation.pending.push((id, receipt)),
                Some(Ok(())) => self.launch_automation(id, cx),
                Some(Err(error)) => self.automation_failed(id, error, cx),
            }
        }
        if self.automation.last_poll.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.automation.last_poll = Instant::now();
        // A failed send barrier or abandoned runtime must not appear to keep running forever.
        let abandoned: Vec<_> = self
            .store
            .scheduler
            .runs
            .iter()
            .filter(|r| {
                r.status == Status::Running
                    && r.session
                        .is_some_and(|id| !self.session_control.runs.contains_key(&id))
            })
            .map(|r| r.id)
            .collect();
        for id in abandoned {
            self.automation_failed(id, label("automation.send_interrupted").into(), cx);
        }
        let now = unix_now();
        let advances = self.store.scheduler.schedules.iter().any(|s| {
            s.enabled && matches!(s.trigger, Trigger::Interval { next_at, .. } if now >= next_at)
        });
        let due = self.store.scheduler.due(now);
        if advances && due.is_empty() {
            self.persist();
        }
        self.save_automation_claims(due, cx);
    }

    pub(super) fn completion_automations(
        &mut self,
        session: u64,
        event: &str,
        failed: bool,
        cancelled: bool,
        cx: &mut Context<Self>,
    ) {
        self.recover_automations();
        let automatic = self
            .store
            .scheduler
            .runs
            .iter()
            .any(|r| r.session == Some(session));
        if self.store.scheduler.finish(session, failed, cancelled) {
            self.persist();
        }
        if automatic || failed || cancelled || !self.writable || self.closing {
            return;
        }
        let Some(workspace) = self
            .store
            .workspaces
            .iter()
            .find(|w| w.sessions.iter().any(|s| s.id == session))
            .map(|w| w.path.clone())
        else {
            return;
        };
        let claims = self.store.scheduler.event(&workspace, event, unix_now());
        // Event consumption must survive restart even when overlap or rate limits suppress it.
        if claims.is_empty() {
            self.persist();
        }
        self.save_automation_claims(claims, cx);
    }

    fn save_automation_claims(&mut self, ids: Vec<u64>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        for id in ids {
            let Some(run) = self.store.scheduler.runs.iter().find(|r| r.id == id) else {
                continue;
            };
            if run.status == Status::Missed {
                self.notify_user(
                    Notice::new(
                        NoticeSource::App,
                        NoticeLevel::Info,
                        label("automation.title"),
                        label("automation.missed_hint"),
                    )
                    .presentation(NoticePresentation::History),
                    cx,
                );
                self.persist();
                continue;
            }
            match self.persist_confirmed() {
                Ok(receipt) => self.automation.pending.push((id, receipt)),
                Err(error) => self.automation_failed(id, error, cx),
            }
        }
        cx.notify();
    }

    fn automation_failed(&mut self, id: u64, error: String, cx: &mut Context<Self>) {
        if let Some(run) = self.store.scheduler.runs.iter_mut().find(|r| r.id == id) {
            run.status = Status::Failed;
            run.detail = error.clone();
        }
        self.notify_user(
            Notice::new(
                NoticeSource::App,
                NoticeLevel::Error,
                label("automation.failed"),
                error,
            )
            .presentation(NoticePresentation::History),
            cx,
        );
        self.persist();
        cx.notify();
    }

    fn launch_automation(&mut self, run_id: u64, cx: &mut Context<Self>) {
        let Some(run) = self
            .store
            .scheduler
            .runs
            .iter()
            .find(|r| r.id == run_id && r.status == Status::Claimed)
        else {
            return;
        };
        let Some(schedule) = self
            .store
            .scheduler
            .schedules
            .iter()
            .find(|s| s.id == run.schedule)
            .cloned()
        else {
            return;
        };
        if !schedule.enabled {
            if let Some(run) = self
                .store
                .scheduler
                .runs
                .iter_mut()
                .find(|r| r.id == run_id)
            {
                run.status = Status::Cancelled;
            }
            self.persist();
            return;
        }
        let preset = schedule.preset;
        let result = (|| {
            if !self.store.cli.enabled(preset.provider) {
                return Err(label("automation.engine_disabled").to_owned());
            }
            let index = self
                .store
                .workspaces
                .iter()
                .position(|w| w.path == preset.workspace)
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            // prepare_session does not change the composer or selected session. The saved preset
            // overrides every route and permission, independent of current composer settings.
            let mut session = self.prepare_session(preset.provider);
            preset.apply(&mut session);
            session.permission = PermissionLevel::ReadOnly;
            let id = session.id;
            self.next_id += 1;
            self.store.workspaces[index].sessions.insert(0, session);
            if let Some(run) = self
                .store
                .scheduler
                .runs
                .iter_mut()
                .find(|r| r.id == run_id)
            {
                run.session = Some(id);
                run.status = Status::Running;
            }
            match self.start_turn(index, Some(id), preset.provider, preset.prompt, true, cx) {
                Ok(_) => {
                    self.sidebar.invalidate_workspace(&preset.workspace);
                    Ok(())
                }
                Err(error) => {
                    self.store.workspaces[index].sessions.retain(|s| s.id != id);
                    if let Some(run) = self
                        .store
                        .scheduler
                        .runs
                        .iter_mut()
                        .find(|r| r.id == run_id)
                    {
                        run.session = None;
                    }
                    Err(error)
                }
            }
        })();
        if let Err(error) = result {
            self.automation_failed(run_id, error, cx);
        }
        cx.notify();
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{AutomationUi, ConduitApp, PermissionLevel, Provider, Store, Workspace};
    use crate::{
        automation::Status,
        providers::{ModelChannel, ReasoningEffort},
        workflow::Preset,
    };
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn failed_claim_save_never_starts_a_model_and_keeps_the_foreground_draft(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let workspace = std::env::temp_dir();
                let store = Store {
                    workspaces: vec![Workspace {
                        path: workspace.clone(),
                        sessions: vec![crate::app::tests::session()],
                    }],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.automation = AutomationUi {
                    recovered: true,
                    ..Default::default()
                };
                app.persistence = crate::persistence::Persistence::with_writer(|_| {
                    Err("fixture write failure".into())
                });
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app.input.update(cx, |input, cx| {
                    input.set_value("keep this draft", window, cx)
                });
                app.store
                    .scheduler
                    .add(
                        Preset {
                            id: 1,
                            workspace,
                            name: "test".into(),
                            prompt: "never send".into(),
                            provider: Provider::Codex,
                            channel: ModelChannel::Cli,
                            model: None,
                            effort: ReasoningEffort::Default,
                            acp_variant: None,
                            permission: PermissionLevel::ReadOnly,
                            archived: false,
                        },
                        Some(3600),
                        100,
                    )
                    .unwrap();
                let claims = app.store.scheduler.due(3700);
                app.save_automation_claims(claims, cx);
                app.persistence.flush();
                app.poll_automations(cx);
                assert!(app.session_control.runs.is_empty());
                assert_eq!(app.store.workspaces[0].sessions.len(), 1);
                assert_eq!(app.input.read(cx).value().as_ref(), "keep this draft");
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.store.scheduler.runs[0].status, Status::Failed);
                app.persistence.flush();
                app.writable = false;
                app
            });
            gpui_kit::component::Root::new(app, window, cx)
        });
    }
}
