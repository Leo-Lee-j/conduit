use super::*;
use crate::{
    notifications::{ApprovalOutcome, Center},
    protocol::{Approval, ApprovalKind},
};
use gpui_kit::{Context, Window};

struct Running {
    events: crate::event_queue::Sender,
    decision: Box<dyn Fn() -> Option<(String, bool)>>,
}

fn start(app: &mut ConduitApp, session: u64) -> Running {
    let (runtime, events, decision) = ChatRuntime::test_approval_channel();
    let path = app.store.workspaces[(session - 1) as usize].path.clone();
    app.session_control.runs.insert(
        session,
        ActiveRun::new(
            runtime,
            path,
            "same-prefix".into(),
            Default::default(),
            None,
            Default::default(),
        ),
    );
    Running {
        events,
        decision: Box::new(decision),
    }
}

fn request(id: &str) -> Approval {
    Approval {
        id: id.into(),
        title: "请求执行命令".into(),
        detail: r#"{"toolInput":{"command":"cargo check --offline","cwd":"D:/work"}}"#.into(),
        wire_id: serde_json::json!(id),
        input: serde_json::json!({"itemId":"tool-1"}),
        allow_once: true,
        kind: ApprovalKind::Command,
    }
}

fn publish(
    app: &mut ConduitApp,
    running: &Running,
    approval: Approval,
    window: &mut Window,
    cx: &mut Context<ConduitApp>,
) -> u64 {
    running.events.send(Event::Approval(approval)).unwrap();
    app.poll_streams(window, cx);
    app.notifications.center.records()[0].id
}

#[gpui_kit::test]
fn background_actions_target_the_exact_ticket_and_wait_for_acknowledgement(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let (first, second, id, focus) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.selected_workspace = Some(0);
            app.selected_session = Some(1);
            app.input.update(cx, |input, cx| {
                input.set_value("保留未发送的草稿", window, cx)
            });
            let first = start(app, 1);
            let second = start(app, 2);
            publish(app, &first, request("approval-2"), window, cx);
            publish(app, &second, request("approval-1"), window, cx);
            let id = publish(app, &second, request("approval-2"), window, cx);
            app.toggle_notifications(window, cx);
            (first, second, id, window.focused(cx))
        })
    });
    for width in [1280., 850.] {
        cx.simulate_resize(size(px(width), px(1000.)));
        draw(cx);
        let footer = cx
            .debug_bounds(format!("notification-footer-{id}").leak())
            .unwrap();
        let card = cx
            .debug_bounds(format!("notification-record-{id}").leak())
            .unwrap();
        let allow = cx
            .debug_bounds(format!("notification-allow-{id}").leak())
            .unwrap();
        let deny = cx
            .debug_bounds(format!("notification-deny-{id}").leak())
            .unwrap();
        let open = cx
            .debug_bounds(format!("notification-open-{id}").leak())
            .unwrap();
        assert!(allow.left() >= footer.left());
        assert!(allow.right() <= deny.left() && deny.right() <= open.left());
        assert_eq!(open.right(), footer.right());
        assert!(footer.left() > card.left() && footer.right() < card.right());
        assert!(
            cx.debug_bounds(format!("notification-read-{id}").leak())
                .is_none()
        );
    }
    click(cx, &format!("notification-allow-{id}"));
    assert_eq!((first.decision)(), None);
    assert_eq!((second.decision)(), Some(("approval-2".into(), true)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(app.selected_session, Some(1));
            assert_eq!(app.selected_workspace, Some(0));
            assert_eq!(app.input.read(cx).value().as_str(), "保留未发送的草稿");
            assert_eq!(window.focused(cx), focus);
            assert!(
                app.session_control.runs[&2]
                    .approvals
                    .is_submitted("approval-2")
            );
            assert!(
                !app.session_control.runs[&2]
                    .approvals
                    .is_submitted("approval-1")
            );
            let record = app.notifications.center.get(id).unwrap();
            assert!(record.message.contains("cargo check --offline"));
            assert!(record.read_at.is_none());
            assert!(record.approval_outcome.is_none());
            assert_eq!(record.lifecycle, Lifecycle::Active);
            let button =
                test_support::find(window, &[], &("notification-allow", id).into()).unwrap();
            assert_eq!(button.disabled(), Some(true));
            assert_eq!(button.label(), Some("审批提交中"));
            assert_eq!(
                test_support::find(window, &[], &("notification-deny", id).into())
                    .unwrap()
                    .disabled(),
                Some(true)
            );
            // The callback itself must reject duplicates, even before another frame.
            app.decide_notification(id, false, window, cx);
            app.decide_notification(id, true, window, cx);
        })
    });
    click(cx, &format!("notification-deny-{id}"));
    assert_eq!((second.decision)(), None);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            second
                .events
                .send(Event::ApprovalResolved {
                    id: "approval-2".into(),
                    allow: true,
                })
                .unwrap();
            app.poll_streams(window, cx);
            let record = app.notifications.center.get(id).unwrap();
            assert_eq!(record.approval_outcome, Some(ApprovalOutcome::Allowed));
            assert!(record.read_at.is_some());
            assert_eq!(record.lifecycle, Lifecycle::Resolved);
            assert_eq!(record.occurrences, 1);
            assert_eq!(app.notifications.center.records().len(), 3);
            assert_eq!(app.session_control.runs[&2].approvals.len(), 1);
            app.decide_notification(id, false, window, cx);
            assert_eq!((second.decision)(), None);
            let restored =
                Center::restore(app.notifications.center.snapshot(), unix_now()).unwrap();
            assert_eq!(
                restored.get(id).unwrap().approval_outcome,
                Some(ApprovalOutcome::Allowed)
            );
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds(format!("notification-allow-{id}").leak())
            .is_none()
    );
}

#[gpui_kit::test]
fn unsupported_allow_once_keeps_deny_and_questions_only_offer_navigation(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let (running, id, question) = cx.update(|window, cx| app.update(cx, |app, cx| {
        let running = start(app, 2);
        let mut approval = request("permission");
        approval.allow_once = false;
        let id = publish(app, &running, approval, window, cx);
        let mut question = request("question");
        question.kind = ApprovalKind::CodexUserInput;
        question.input = serde_json::json!({"questions":[{"id":"q", "question":"选择方案", "options":[{"label":"方案 A"}]}]});
        let question = publish(app, &running, question, window, cx);
        app.toggle_notifications(window, cx);
        (running, id, question)
    }));
    draw(cx);
    cx.update(|window, _| {
        assert_eq!(
            test_support::find(window, &[], &("notification-allow", id).into())
                .unwrap()
                .disabled(),
            Some(true)
        );
        assert!(
            !test_support::find(window, &[], &("notification-deny", id).into())
                .unwrap()
                .disabled()
                .unwrap_or(false)
        );
    });
    assert!(
        cx.debug_bounds(format!("notification-allow-{question}").leak())
            .is_none()
    );
    assert!(
        cx.debug_bounds(format!("notification-deny-{question}").leak())
            .is_none()
    );
    assert!(
        cx.debug_bounds(format!("notification-open-{question}").leak())
            .is_some()
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.decide_notification(question, true, window, cx);
            app.decide_notification(question, false, window, cx);
            app.decide_notification(id, true, window, cx);
            assert_eq!((running.decision)(), None);
        })
    });
    click(cx, &format!("notification-deny-{id}"));
    assert_eq!((running.decision)(), Some(("permission".into(), false)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            running
                .events
                .send(Event::ApprovalResolved {
                    id: "permission".into(),
                    allow: false,
                })
                .unwrap();
            app.poll_streams(window, cx);
            assert_eq!(
                app.notifications.center.get(id).unwrap().approval_outcome,
                Some(ApprovalOutcome::Denied)
            );
            assert!(app.notifications.center.get(id).unwrap().read_at.is_some());
            assert!(
                app.session_control.runs[&2]
                    .approvals
                    .get("question")
                    .is_some()
            );
        })
    });
}

#[gpui_kit::test]
fn chat_submission_refreshes_cached_notification_controls(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let (running, id) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let running = start(app, 1);
            let id = publish(app, &running, request("approval-1"), window, cx);
            app.toggle_notifications(window, cx);
            (running, id)
        })
    });
    draw(cx);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.decide(1, "approval-1".into(), false, window, cx)
        })
    });
    draw(cx);
    assert_eq!((running.decision)(), Some(("approval-1".into(), false)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            assert_eq!(
                test_support::find(window, &[], &("notification-deny", id).into())
                    .unwrap()
                    .disabled(),
                Some(true)
            );
            app.decide_notification(id, true, window, cx);
            assert_eq!((running.decision)(), None);
        })
    });
}

#[gpui_kit::test]
fn stale_callbacks_reject_cancelled_finished_deleted_changed_and_closed_targets(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    for scenario in [
        "request-cancelled",
        "run-cancelled",
        "finished",
        "deleted",
        "session-replaced",
        "workspace-changed",
        "closing",
        "restored",
    ] {
        let (app, cx) = fixture(cx, true);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let running = start(app, 2);
                let id = publish(app, &running, request("approval-1"), window, cx);
                match scenario {
                    "request-cancelled" => {
                        running
                            .events
                            .send(Event::ApprovalCancelled("approval-1".into()))
                            .unwrap();
                        app.poll_streams(window, cx);
                    }
                    "run-cancelled" => app.session_control.runs.get_mut(&2).unwrap().cancel(),
                    "finished" => {
                        running
                            .events
                            .send(Event::Finished {
                                error: None,
                                cancelled: true,
                            })
                            .unwrap();
                        app.poll_streams(window, cx);
                    }
                    "deleted" => {
                        app.session_control.runs.remove(&2);
                        app.delete_session(2, cx);
                    }
                    "session-replaced" => app.store.workspaces[1].sessions[0].created_at += 1,
                    "workspace-changed" => app.store.workspaces[1].path.push("different-workspace"),
                    "closing" => app.closing = true,
                    "restored" => {
                        app.notifications.center =
                            Center::restore(app.notifications.center.snapshot(), unix_now())
                                .unwrap();
                        app.notifications.approvals.clear();
                    }
                    _ => unreachable!(),
                }
                // No redraw/poll between invalidation and the stale callback.
                app.decide_notification(id, true, window, cx);
                app.decide_notification(id, false, window, cx);
                assert_eq!((running.decision)(), None, "{scenario}");
                assert!(app.notification_approval_state(id).is_none(), "{scenario}");
                assert_eq!(
                    app.notifications.center.get(id).unwrap().lifecycle,
                    Lifecycle::Resolved,
                    "{scenario}"
                );
                assert!(app.notifications.approvals.is_empty(), "{scenario}");
            })
        });
    }
}

#[gpui_kit::test]
fn repeated_prefix_and_ticket_cannot_rebind_an_old_notification(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let first = start(app, 2);
            let old = publish(app, &first, request("approval-1"), window, cx);
            let second = start(app, 2);
            let new = publish(app, &second, request("approval-1"), window, cx);
            assert_ne!(old, new);
            assert_eq!(
                app.notifications.center.get(old).unwrap().approval_outcome,
                Some(ApprovalOutcome::Expired)
            );
            app.decide_notification(old, true, window, cx);
            assert_eq!((first.decision)(), None);
            assert_eq!((second.decision)(), None);
            app.decide_notification(new, true, window, cx);
            assert_eq!((second.decision)(), Some(("approval-1".into(), true)));
            // Restored history has neither active capabilities nor authority over a
            // later runtime that uses the same transcript prefix and local ticket.
            app.notifications.center =
                Center::restore(app.notifications.center.snapshot(), unix_now()).unwrap();
            app.notifications.approvals.clear();
            let third = start(app, 2);
            let restored_new = publish(app, &third, request("approval-1"), window, cx);
            assert_ne!(restored_new, new);
            app.decide_notification(old, true, window, cx);
            app.decide_notification(new, true, window, cx);
            assert_eq!((third.decision)(), None);
            app.decide_notification(restored_new, false, window, cx);
            assert_eq!((third.decision)(), Some(("approval-1".into(), false)));
        })
    });
}

#[gpui_kit::test]
fn repeated_question_ticket_notifies_again_and_resolves_only_its_runtime(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let mut question = request("approval-1");
            question.kind = ApprovalKind::CodexUserInput;
            question.input = serde_json::json!({"questions":[{"id":"q","question":"选择方案","options":[{"label":"A"}]}]});
            let first = start(app, 2);
            let old = publish(app, &first, question.clone(), window, cx);
            app.notifications.center.mark_read(old, unix_now());
            first.events.send(Event::ApprovalResolved { id: question.id.clone(), allow: true }).unwrap();
            app.poll_streams(window, cx);
            assert_eq!(app.notifications.center.get(old).unwrap().lifecycle, Lifecycle::Resolved);
            let second = start(app, 2);
            let new = publish(app, &second, question.clone(), window, cx);
            assert_ne!(old, new);
            let record = app.notifications.center.get(new).unwrap();
            assert_eq!(record.lifecycle, Lifecycle::Active);
            assert!(record.read_at.is_none());
            assert!(app.notifications.pending.contains(&new));
            assert!(app.notification_approval_state(new).is_none());
            second.events.send(Event::ApprovalCancelled(question.id)).unwrap();
            app.poll_streams(window, cx);
            assert_eq!(app.notifications.center.get(new).unwrap().lifecycle, Lifecycle::Resolved);
            assert!(app.notifications.center.get(old).unwrap().read_at.is_some());
        });
    });
}

#[gpui_kit::test]
fn disconnected_runtime_expires_approval_instead_of_sticking_in_submitting(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let running = start(app, 2);
            let id = publish(app, &running, request("approval-1"), window, cx);
            drop(running.decision);
            app.decide_notification(id, true, window, cx);
            assert!(app.notification_approval_state(id).is_none());
            let record = app.notifications.center.get(id).unwrap();
            assert_eq!(record.approval_outcome, Some(ApprovalOutcome::Expired));
            assert!(record.read_at.is_none());
            assert!(app.session_control.runs[&2].status.contains("提交失败"));
        })
    });
}

#[gpui_kit::test]
fn cancellation_refreshes_cached_controls_and_does_not_claim_approval(cx: &mut TestAppContext) {
    let _locale = crate::test_support::chinese();
    let (app, cx) = fixture(cx, true);
    let (running, id) = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            let running = start(app, 2);
            let id = publish(app, &running, request("approval-1"), window, cx);
            app.toggle_notifications(window, cx);
            (running, id)
        })
    });
    click(cx, &format!("notification-allow-{id}"));
    assert_eq!((running.decision)(), Some(("approval-1".into(), true)));
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            running
                .events
                .send(Event::ApprovalCancelled("approval-1".into()))
                .unwrap();
            app.poll_streams(window, cx);
            let record = app.notifications.center.get(id).unwrap();
            assert_eq!(record.approval_outcome, Some(ApprovalOutcome::Cancelled));
            assert!(record.read_at.is_none());
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds(format!("notification-allow-{id}").leak())
            .is_none()
    );
    let next = cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            publish(app, &running, request("approval-2"), window, cx)
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds(format!("notification-allow-{next}").leak())
            .is_some()
    );
    cx.update(|window, cx| {
        app.update(cx, |app, cx| {
            app.session_control.runs.get_mut(&2).unwrap().cancel();
            app.poll_notifications(window, cx);
        })
    });
    draw(cx);
    assert!(
        cx.debug_bounds(format!("notification-allow-{next}").leak())
            .is_none()
    );
}
