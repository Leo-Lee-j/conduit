use super::*;
use crate::domain::{QuestionRecord, ToolApproval};
use crate::protocol::{Approval, UserQuestion};
use gpui_kit::component::{checkbox::Checkbox, radio::Radio};

pub(super) fn question_record(question: &UserQuestion) -> QuestionRecord {
    QuestionRecord {
        id: question.id.clone(),
        header: question.header.clone(),
        question: question.question.clone(),
        options: question.options.clone(),
        multiple: question.multiple,
        secret: question.secret,
        selected: vec![],
        answers: vec![],
    }
}

pub(super) struct QuestionDraft {
    pub(super) question: UserQuestion,
    pub(super) selected: HashSet<usize>,
    pub(super) custom: Entity<InputState>,
    pub(super) _subscription: Subscription,
}

pub(super) fn history(item: &ChatItem) -> Option<&ToolApproval> {
    item.approval
        .as_ref()
        .filter(|approval| !approval.questions.is_empty())
}

fn answer_text(question: &QuestionRecord, approval: &ToolApproval) -> String {
    use crate::message_status::ApprovalStatus;
    if approval.status != ApprovalStatus::Answered {
        return approval.status.label().to_string();
    }
    if question.secret {
        conduit::i18n::text("user_input.hidden_answer")
    } else {
        question.answers.join(" · ")
    }
}

/// Selected options already show the answer; reserve the detail for typed text.
fn answer_detail(question: &QuestionRecord, approval: &ToolApproval) -> Option<String> {
    if approval.status != crate::message_status::ApprovalStatus::Answered || question.secret {
        return Some(answer_text(question, approval));
    }
    let mut answers = question.answers.clone();
    for index in &question.selected {
        if let Some((label, _)) = question.options.get(*index)
            && let Some(position) = answers.iter().position(|answer| answer == label)
        {
            // Remove one occurrence so a typed answer matching an option survives.
            answers.remove(position);
        }
    }
    (!answers.is_empty()).then(|| answers.join(" · "))
}

pub(super) fn history_text(approval: &ToolApproval) -> String {
    let mut text = Vec::new();
    for (index, question) in approval.questions.iter().enumerate() {
        text.push(format!("{}. {}", index + 1, question.question));
        if !question.header.is_empty() {
            text.push(question.header.clone());
        }
        for (index, (label, description)) in question.options.iter().enumerate() {
            text.push(format!(
                "[{}] {label}{}",
                if question.selected.contains(&index) {
                    "x"
                } else {
                    " "
                },
                if description.is_empty() {
                    String::new()
                } else {
                    format!(" — {description}")
                }
            ));
        }
        text.push(format!(
            "{}: {}\n",
            conduit::i18n::text_static("user_input.your_answer"),
            answer_text(question, approval)
        ));
    }
    text.join("\n")
}

/// Natural-height question content shared by the transcript and details dock.
pub(super) fn history_content(scope: &str, approval: &ToolApproval) -> AnyElement {
    let mut content = v_flex()
        .w_full()
        .min_w_0()
        .p_4()
        .gap_4()
        .text_size(px(13.))
        .line_height(relative(1.5))
        .text_color(rgb(palette().text));
    for (question_index, question) in approval.questions.iter().enumerate() {
        let mut card = v_flex()
            .w_full()
            .min_w_0()
            .gap_3()
            .when(question_index > 0, |card| {
                card.pt_4().border_t_1().border_color(rgb(palette().border))
            })
            .when(!question.header.is_empty(), |card| {
                card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(format!("{:02}  {}", question_index + 1, question.header)),
                )
            })
            .child(
                div()
                    .font_semibold()
                    .child(gpui_kit::base::SelectableText::new(
                        SharedString::from(format!("{scope}-question-{question_index}")),
                        question.question.clone(),
                    )),
            );
        let mut options = v_flex().w_full().min_w_0().gap_2();
        for (option_index, (label, description)) in question.options.iter().enumerate() {
            let selected = question.selected.contains(&option_index);
            options = options.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .pt(px(2.))
                            .text_color(rgb(if selected {
                                palette().activity_running
                            } else {
                                palette().faint
                            }))
                            .child(
                                Icon::new(if selected {
                                    FullIconName::CircleCheck
                                } else {
                                    FullIconName::Circle
                                })
                                .small(),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .when(selected, |label| label.font_medium())
                                    .child(label.clone()),
                            )
                            .when(!description.is_empty(), |option| {
                                option.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette().muted))
                                        .child(description.clone()),
                                )
                            }),
                    ),
            );
        }
        card = card
            .child(options)
            .when_some(answer_detail(question, approval), |card, answer| {
                card.child(
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .p_3()
                        .gap_1()
                        .rounded(px(8.))
                        .bg(rgb(palette().surface_strong))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(conduit::i18n::text_static("user_input.your_answer")),
                        )
                        .child(gpui_kit::base::SelectableText::new(
                            SharedString::from(format!("{scope}-answer-{question_index}")),
                            answer,
                        )),
                )
            });
        content = content.child(card);
    }
    content.into_any_element()
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{ActiveRun, ChatRuntime, ConduitApp};
    use crate::{
        domain::{SessionRecord, Workspace},
        protocol::{Event, Protocol},
        store::Store,
    };
    use gpui_kit::{
        AppContext as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size,
    };

    #[gpui_kit::test]
    fn cli_user_input_requires_choices_and_preserves_drafts_until_resolution(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let store = Store {
                    workspaces: vec![Workspace {
                        path: std::env::temp_dir(),
                        sessions: vec![SessionRecord {
                            subagents: Default::default(),
                            is_subagent: false,
                            branching: Default::default(),
                            transcript: Default::default(),
                            id: 1,
                            provider: crate::domain::Provider::Codex,
                            title: "Question test".into(),
                            native_id: None,
                            created_at: 0,
                            exit_code: None,
                            archived: false,
                            pinned: false,
                            model: None,
                            messages: vec![],
                            interrupted: false,
                            channel: Default::default(),
                            reasoning_effort: Default::default(),
                            opencode: Default::default(),
                            permission: crate::domain::PermissionLevel::FullAccess,
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
        let mut protocol = Protocol::new(
            crate::domain::Provider::Codex,
            "C:/work".into(),
            "test".into(),
            None,
        );
        let step = protocol.accept(serde_json::json!({"id":"question-1","method":"item/tool/requestUserInput","params":{"questions":[
            {"id":"choice","header":"方案","question":"选择方案？","isOther":false,"options":[{"label":"A","description":"第一项"},{"label":"B","description":"第二项"}]},
            {"id":"name","header":"名称","question":"项目名称？","isOther":true,"options":null}
        ]}}));
        let (runtime, sender) = ChatRuntime::test_channel();
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                this.session_control.runs.insert(
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
                assert_eq!(this.session_control.runs[&1].status, "等待你的回答");
                assert!(
                    this.session_control.runs[&1].question_drafts["question-1"]
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
        let click = |cx: &mut VisualTestContext, selector: &'static str| {
            let center = cx.debug_bounds(selector).unwrap().center();
            cx.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
            cx.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
            cx.run_until_parked();
        };
        click(cx, "cli-question-submit");
        cx.update(|_, cx| {
            assert!(
                !app.read(cx).session_control.runs[&1]
                    .approvals
                    .is_submitted("question-1")
            )
        });
        click(cx, "cli-option-0-1");
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                let draft = &this.session_control.runs[&1].question_drafts["question-1"][0];
                assert_eq!(draft.answers(cx), vec!["B"]);
                this.session_control.runs[&1].question_drafts["question-1"][1]
                    .custom
                    .update(cx, |input, cx| input.set_value("中文项目", window, cx));
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        click(cx, "cli-question-submit");
        cx.update(|window, cx| {
            app.update(cx, |this, cx| {
                assert!(
                    this.session_control.runs[&1]
                        .approvals
                        .is_submitted("question-1")
                );
                assert_eq!(
                    this.session_control.runs[&1].question_drafts["question-1"][1].answers(cx),
                    vec!["中文项目"]
                );
                let draft = &mut this
                    .session_control
                    .runs
                    .get_mut(&1)
                    .unwrap()
                    .question_drafts
                    .get_mut("question-1")
                    .unwrap()[1];
                draft.question.secret = true;
                let hidden = draft.record(cx);
                assert!(hidden.answers.is_empty());
                assert!(hidden.selected.is_empty());
                assert!(!serde_json::to_string(&hidden).unwrap().contains("中文项目"));
                draft.question.secret = false;
                sender
                    .send(Event::ApprovalResolved {
                        id: "question-1".into(),
                        allow: true,
                    })
                    .unwrap();
                this.poll(window, cx);
                assert!(this.session_control.runs[&1].question_drafts.is_empty());
                assert!(this.session_control.runs[&1].approvals.is_empty());
                let record = this.session(1).unwrap().messages.last().unwrap();
                let history = record.approval.as_ref().unwrap();
                assert_eq!(history.questions.len(), 2);
                assert_eq!(history.questions[0].question, "选择方案？");
                assert_eq!(history.questions[0].options.len(), 2);
                assert_eq!(history.questions[0].selected, vec![1]);
                assert_eq!(history.questions[0].answers, vec!["B"]);
                assert_eq!(history.questions[1].answers, vec!["中文项目"]);
                let restored: crate::domain::ChatItem =
                    serde_json::from_str(&serde_json::to_string(record).unwrap()).unwrap();
                assert_eq!(
                    restored.approval.as_ref().unwrap().questions,
                    history.questions
                );
                let copied = super::history_text(history);
                assert!(copied.contains("[ ] A — 第一项"));
                assert!(copied.contains("[x] B — 第二项"));
                assert!(copied.contains("中文项目"));
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
    pub(super) fn record(&self, cx: &App) -> QuestionRecord {
        let mut record = question_record(&self.question);
        if !record.secret {
            record.answers = self.answers(cx);
            record.selected = (0..self.question.options.len())
                .filter(|index| self.selected.contains(index))
                .filter(|_| {
                    self.question.multiple || self.custom.read(cx).value().trim().is_empty()
                })
                .collect();
        }
        record
    }

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
    pub(super) fn set_question_history_collapsed(&mut self, collapsed: bool) {
        self.store.ui.auto_collapse_question_history = collapsed;
        self.chat_rows.invalidate();
        self.scroll.remeasure_items(0..self.scroll.item_count());
    }

    pub(super) fn render_user_input(
        &self,
        approval: &Approval,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.selected_session.unwrap();
        let run = &self.session_control.runs[&session];
        let submitted = run.approvals.is_submitted(&approval.id);
        let disabled = self.closing || submitted || run.runtime.is_cancelled();
        let mut column = v_flex()
            .id("cli-user-input")
            .debug_selector(|| "cli-user-input".into())
            .w_full()
            .gap_4()
            .p_4()
            .rounded(px(12.))
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .text_color(rgb(palette().text))
            .child(div().font_semibold().child(if submitted {
                conduit::i18n::text_static("user_input.submitting_answer")
            } else {
                conduit::i18n::text_static("user_input.the_cli_needs_you_to_confirm_options")
            }));
        if run.approvals.len() > 1 {
            column = column.child(div().text_xs().text_color(rgb(palette().muted)).child(
                conduit::i18n::format(
                    "user_input.more_pending",
                    &[("arg0", format!("{}", run.approvals.len() - 1))],
                ),
            ));
        }
        let mut complete = false;
        let mut progress = String::new();
        if let Some(drafts) = run.question_drafts.get(&approval.id) {
            complete = drafts.iter().all(|draft| !draft.answers(cx).is_empty());
            progress = conduit::i18n::format(
                "user_input.answer_progress",
                &[
                    (
                        "answered",
                        drafts
                            .iter()
                            .filter(|draft| !draft.answers(cx).is_empty())
                            .count()
                            .to_string(),
                    ),
                    ("total", drafts.len().to_string()),
                ],
            );
            let mut body = v_flex()
                .id("cli-questions-body")
                .gap_4()
                .max_h(px(
                    (f32::from(window.viewport_size().height) * 0.4).clamp(120., 360.)
                ))
                .overflow_y_scroll();
            for (question_index, draft) in drafts.iter().enumerate() {
                let question = &draft.question;
                let mut card = v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_3()
                    .when(!question.header.is_empty(), |card| {
                        card.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette().muted))
                                .child(format!("{:02}  {}", question_index + 1, question.header)),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_semibold()
                            .line_height(relative(1.5))
                            .child(question.question.clone()),
                    )
                    .when(!question.options.is_empty(), |card| {
                        card.child(div().text_xs().text_color(rgb(palette().muted)).child(
                            conduit::i18n::text_static(if question.multiple {
                                "user_input.select_multiple_options"
                            } else {
                                "user_input.select_one_option"
                            }),
                        ))
                    });
                let mut options = v_flex().w_full().min_w_0().gap_2();
                for (option_index, (label, description)) in question.options.iter().enumerate() {
                    let selected = draft.selected.contains(&option_index)
                        && (question.multiple || draft.custom.read(cx).value().trim().is_empty());
                    let request = approval.id.clone();
                    let on_change = cx.listener(move |this, checked: &bool, window, cx| {
                        if this.closing {
                            return;
                        }
                        let Some(run) = this.session_control.runs.get_mut(&session) else {
                            return;
                        };
                        if run.runtime.is_cancelled()
                            || run.approvals.is_submitted(&request)
                            || run.approvals.front().is_none_or(|a| a.id != request)
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
                            draft
                                .custom
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        if *checked {
                            draft.selected.insert(option_index);
                        } else {
                            draft.selected.remove(&option_index);
                        }
                        cx.notify();
                    });
                    let content = v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .text_size(px(13.))
                        .line_height(relative(1.5))
                        .child(
                            div()
                                .font_medium()
                                .text_color(rgb(palette().text))
                                .child(label.clone()),
                        )
                        .when(!description.is_empty(), |row| {
                            row.child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette().muted))
                                    .child(description.clone()),
                            )
                        });
                    let control_id =
                        SharedString::from(format!("cli-choice-{question_index}-{option_index}"));
                    let accessible = if description.is_empty() {
                        label.clone()
                    } else {
                        format!("{label}: {description}")
                    };
                    let border = rgb(if selected {
                        palette().activity_running
                    } else {
                        palette().border
                    });
                    let control = if question.multiple {
                        Checkbox::new(control_id)
                            .accessibility_label(accessible)
                            .checked(selected)
                            .disabled(disabled)
                            .w_full()
                            .min_w_0()
                            .p_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(border)
                            .bg(rgb(palette().surface))
                            .when(!disabled, |row| {
                                row.cursor_pointer()
                                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                            })
                            .child(content)
                            .on_change(on_change)
                            .into_any_element()
                    } else {
                        Radio::new(control_id)
                            .accessibility_label(accessible)
                            .checked(selected)
                            .disabled(disabled)
                            .w_full()
                            .min_w_0()
                            .p_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(border)
                            .bg(rgb(palette().surface))
                            .when(!disabled, |row| {
                                row.cursor_pointer()
                                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                            })
                            .child(content)
                            .on_change(on_change)
                            .into_any_element()
                    };
                    options = options.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .debug_selector(move || {
                                format!("cli-option-{question_index}-{option_index}")
                            })
                            .child(control),
                    );
                }
                card = card.child(options);
                if question.custom {
                    card = card.child(
                        v_flex()
                            .gap_2()
                            .child(div().text_xs().text_color(rgb(palette().muted)).child(
                                conduit::i18n::text_static(if question.options.is_empty() {
                                    "user_input.your_answer"
                                } else {
                                    "user_input.other_answer"
                                }),
                            ))
                            .child(
                                Input::new(&draft.custom)
                                    .aria_label(conduit::i18n::text_static(
                                        "user_input.your_answer",
                                    ))
                                    .disabled(disabled),
                            ),
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
                .pt_3()
                .border_t_1()
                .border_color(rgb(palette().border))
                .justify_end()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(rgb(palette().muted))
                        .child(progress),
                )
                .child(
                    Button::new("cli-question-skip")
                        .label(conduit::i18n::text_static("user_input.skip"))
                        .ghost()
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.decide(session, skip_id.clone(), false, window, cx)
                        })),
                )
                .child(
                    Button::new("cli-question-submit")
                        .debug_selector(|| "cli-question-submit".into())
                        .label(conduit::i18n::text_static("user_input.submit_answer"))
                        .primary()
                        .disabled(disabled || !complete)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if this.closing {
                                return;
                            }
                            let Some(run) = this.session_control.runs.get_mut(&session) else {
                                return;
                            };
                            if run.runtime.is_cancelled()
                                || run.approvals.front().is_none_or(|a| a.id != submit_id)
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
                            run.status =
                                conduit::i18n::text_static("user_input.submitting_answer").into();
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

#[cfg(test)]
mod history_tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn copied_history_keeps_options_custom_answers_and_resolution_state() {
        let _locale = crate::test_support::chinese();
        let mut approval = ToolApproval {
            request: "request".into(),
            status: "已回答".into(),
            questions: vec![QuestionRecord {
                id: "choice".into(),
                header: "方案".into(),
                question: "选择方案？".into(),
                options: vec![("A".into(), "第一项".into()), ("B".into(), "第二项".into())],
                multiple: true,
                secret: false,
                selected: vec![1],
                answers: vec!["B".into(), "自定义答案".into()],
            }],
        };
        let text = history_text(&approval);
        for expected in [
            "选择方案？",
            "[ ] A — 第一项",
            "[x] B — 第二项",
            "你的回答: B · 自定义答案",
        ] {
            assert!(text.contains(expected), "{text}");
        }
        approval.questions[0].secret = true;
        let text = history_text(&approval);
        assert!(text.contains("内容已隐藏"));
        assert!(!text.contains("自定义答案"));
        approval.questions[0].secret = false;
        for status in ["已跳过", "已取消", "等待中"] {
            approval.status = status.into();
            let text = history_text(&approval);
            assert!(text.contains(status));
            assert!(!text.contains("自定义答案"));
        }
    }
}
