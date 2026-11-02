use super::*;
use crate::protocol::{Approval, UserQuestion};

pub(super) struct QuestionDraft {
    pub(super) question: UserQuestion,
    pub(super) selected: HashSet<usize>,
    pub(super) custom: Entity<InputState>,
    pub(super) _subscription: Subscription,
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::*;
    use crate::{
        domain::{SessionRecord, Workspace},
        protocol::{Event, Protocol},
        store::Store,
    };

    #[gpui_kit::test]
    fn cli_user_input_requires_choices_and_preserves_drafts_until_resolution(
        cx: &mut TestAppContext,
    ) {
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir(),
                        sessions: vec![SessionRecord {
                            id: 1,
                            provider: Provider::Codex,
                            title: "Question test".into(),
                            native_id: None,
                            created_at: 0,
                            exit_code: None,
                            archived: false,
                            model: None,
                            messages: vec![],
                            interrupted: false,
                            channel: Default::default(),
                            reasoning_effort: Default::default(),
                            opencode: Default::default(),
                            permission: PermissionLevel::FullAccess,
                            usage: Default::default(),
                        }],
                    }],
                    ..Default::default()
                };
                let mut app = ConduitApp::with_store(window, cx, Ok(store), false, false);
                app.writable = false;
                app.selected_workspace = Some(0);
                app.selected_session = Some(1);
                app
            });
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        let mut protocol = Protocol::new(Provider::Codex, "C:/work".into(), "test".into(), None);
        let step = protocol.accept(serde_json::json!({"id":"question-1","method":"item/tool/requestUserInput","params":{"questions":[
            {"id":"choice","header":"方案","question":"选择方案？","isOther":false,"options":[{"label":"A","description":"第一项"},{"label":"B","description":"第二项"}]},
            {"id":"name","header":"名称","question":"项目名称？","isOther":true,"options":null}
        ]}}));
        let (runtime, sender) = ChatRuntime::test_channel();
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                this.runs.insert(
                    1,
                    ActiveRun::new(
                        runtime,
                        std::env::temp_dir(),
                        "turn-0".into(),
                        Default::default(),
                        None,
                        Default::default(),
                    ),
                );
                sender
                    .send(step.events.into_iter().next().unwrap())
                    .unwrap();
                this.poll(window, cx);
                assert_eq!(this.runs[&1].status, "等待你的回答");
                assert!(
                    this.runs[&1].question_drafts["question-1"]
                        .iter()
                        .all(|d| d.answers(cx).is_empty())
                );
            })
        });
        cx.simulate_resize(size(px(1280.), px(900.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("cli-user-input").is_some());
        let click = |cx: &mut VisualTestContext, selector: &str| {
            let center = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
        };
        click(&mut cx, "cli-question-submit");
        cx.update(|_, cx| assert!(!app.read(cx).runs[&1].approvals.is_submitted("question-1")));
        click(&mut cx, "cli-option-0-1");
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                let draft = &this.runs[&1].question_drafts["question-1"][0];
                assert_eq!(draft.answers(cx), vec!["B"]);
                this.runs[&1].question_drafts["question-1"][1]
                    .custom
                    .update(cx, |input, cx| input.set_value("中文项目", window, cx));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        click(&mut cx, "cli-question-submit");
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                assert!(this.runs[&1].approvals.is_submitted("question-1"));
                assert_eq!(
                    this.runs[&1].question_drafts["question-1"][1].answers(cx),
                    vec!["中文项目"]
                );
                sender
                    .send(Event::ApprovalResolved {
                        id: "question-1".into(),
                        allow: true,
                    })
                    .unwrap();
                this.poll(window, cx);
                assert!(this.runs[&1].question_drafts.is_empty());
                assert!(this.runs[&1].approvals.is_empty());
                assert_eq!(
                    this.session(1)
                        .unwrap()
                        .messages
                        .last()
                        .unwrap()
                        .approval
                        .as_ref()
                        .unwrap()
                        .status,
                    "已回答"
                );
            })
        });
    }
}

impl QuestionDraft {
    fn answers(&self, cx: &App) -> Vec<String> {
        let custom = self.custom.read(cx).value().to_string();
        if !self.question.multiple && !custom.trim().is_empty() {
            return vec![custom.trim().into()];
        }
        let mut answers: Vec<_> = self
            .question
            .options
            .iter()
            .enumerate()
            .filter(|(index, _)| self.selected.contains(index))
            .map(|(_, (label, _))| label.clone())
            .collect();
        if !custom.trim().is_empty() {
            answers.push(custom.trim().into());
        }
        answers
    }
}

impl ConduitApp {
    pub(super) fn render_user_input(
        &self,
        approval: &Approval,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.selected_session.unwrap();
        let run = &self.runs[&session];
        let submitted = run.approvals.is_submitted(&approval.id);
        let disabled = self.closing || submitted || run.runtime.is_cancelled();
        let mut column = v_flex()
            .id("cli-user-input")
            .debug_selector(|| "cli-user-input".into())
            .w_full()
            .gap_3()
            .p_4()
            .rounded(px(14.))
            .border_1()
            .border_color(rgb(palette().warning_border))
            .bg(rgb(palette().warning_bg))
            .child(div().font_semibold().child(if submitted {
                "正在提交回答…"
            } else {
                "CLI 需要你确认选项"
            }));
        if run.approvals.len() > 1 {
            column = column.child(
                div()
                    .text_xs()
                    .child(format!("还有 {} 项待处理", run.approvals.len() - 1)),
            );
        }
        let mut complete = false;
        if let Some(drafts) = run.question_drafts.get(&approval.id) {
            complete = drafts.iter().all(|draft| !draft.answers(cx).is_empty());
            let mut body = v_flex()
                .id("cli-questions-body")
                .gap_3()
                .max_h(px(
                    (f32::from(window.viewport_size().height) * 0.4).clamp(120., 360.)
                ))
                .overflow_y_scroll();
            for (question_index, draft) in drafts.iter().enumerate() {
                let question = &draft.question;
                let mut card = v_flex()
                    .gap_2()
                    .when(!question.header.is_empty(), |card| {
                        card.child(div().text_xs().child(question.header.clone()))
                    })
                    .child(div().text_sm().child(question.question.clone()))
                    .when(question.multiple, |card| {
                        card.child(div().text_xs().child("可选择多项"))
                    });
                for (option_index, (label, description)) in question.options.iter().enumerate() {
                    let selected = draft.selected.contains(&option_index);
                    let selected = selected
                        && (question.multiple || draft.custom.read(cx).value().trim().is_empty());
                    let request = approval.id.clone();
                    card = card.child(
                        div()
                            .id(format!("cli-option-{question_index}-{option_index}"))
                            .debug_selector(move || {
                                format!("cli-option-{question_index}-{option_index}")
                            })
                            .p_2()
                            .rounded_md()
                            .border_1()
                            .border_color(rgb(if selected {
                                palette().warning_accent
                            } else {
                                palette().border
                            }))
                            .when(!disabled, |row| row.cursor_pointer())
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(if selected { "●" } else { "○" })
                                    .child(label.clone()),
                            )
                            .when(!description.is_empty(), |row| {
                                row.child(div().text_xs().child(description.clone()))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.closing {
                                    return;
                                }
                                let Some(run) = this.runs.get_mut(&session) else {
                                    return;
                                };
                                if run.runtime.is_cancelled()
                                    || run.approvals.is_submitted(&request)
                                    || !run.approvals.front().is_some_and(|a| a.id == request)
                                {
                                    return;
                                }
                                let Some(draft) = run
                                    .question_drafts
                                    .get_mut(&request)
                                    .and_then(|d| d.get_mut(question_index))
                                else {
                                    return;
                                };
                                if !draft.question.multiple {
                                    draft.selected.clear();
                                }
                                if !draft.selected.insert(option_index) {
                                    draft.selected.remove(&option_index);
                                }
                                draft
                                    .custom
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                cx.notify();
                            })),
                    );
                }
                if question.custom {
                    card = card.child(
                        Input::new(&draft.custom)
                            .aria_label("其他回答")
                            .disabled(disabled),
                    );
                }
                body = body.child(card);
            }
            column = column.child(body);
        }
        let skip_id = approval.id.clone();
        let submit_id = approval.id.clone();
        column = column.child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("cli-question-skip")
                        .label("跳过")
                        .small()
                        .ghost()
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.decide(session, skip_id.clone(), false, window, cx)
                        })),
                )
                .child(
                    Button::new("cli-question-submit")
                        .debug_selector(|| "cli-question-submit".into())
                        .label("提交回答")
                        .small()
                        .primary()
                        .disabled(disabled || !complete)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.closing {
                                return;
                            }
                            let Some(run) = this.runs.get_mut(&session) else {
                                return;
                            };
                            if run.runtime.is_cancelled()
                                || !run.approvals.front().is_some_and(|a| a.id == submit_id)
                            {
                                return;
                            }
                            let Some(drafts) = run.question_drafts.get(&submit_id) else {
                                return;
                            };
                            let answers: BTreeMap<_, _> = drafts
                                .iter()
                                .map(|d| (d.question.id.clone(), d.answers(cx)))
                                .collect();
                            if answers.values().any(Vec::is_empty)
                                || !run.approvals.begin(&submit_id)
                            {
                                return;
                            }
                            run.runtime.answer(submit_id.clone(), answers);
                            run.status = "正在提交回答…".into();
                            this.input.read(cx).focus_handle(cx).focus(window, cx);
                            cx.notify();
                        })),
                ),
        );
        div()
            .w_full()
            .px_6()
            .mb_4()
            .child(column.when(!self.store.ui.full_width_content, |card| {
                card.max_w(px(840.)).mx_auto()
            }))
            .into_any_element()
    }
}
