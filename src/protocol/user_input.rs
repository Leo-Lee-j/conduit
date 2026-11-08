//! Questions need explicit answers even when tool permissions are bypassed.
use super::*;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct UserQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<(String, String)>,
    pub multiple: bool,
    pub custom: bool,
    pub secret: bool,
}

impl Approval {
    pub fn is_user_input(&self) -> bool {
        matches!(
            self.kind,
            ApprovalKind::CodexUserInput
                | ApprovalKind::ClaudeUserInput
                | ApprovalKind::OpenCodeUserInput
        )
    }
}

impl Approval {
    pub fn questions(&self) -> Vec<UserQuestion> {
        if !self.is_user_input() {
            return vec![];
        }
        self.input["questions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, q)| {
                let claude = self.kind == ApprovalKind::ClaudeUserInput;
                let opencode = self.kind == ApprovalKind::OpenCodeUserInput;
                UserQuestion {
                    id: if opencode {
                        index.to_string()
                    } else {
                        text(q, if claude { "question" } else { "id" }).into()
                    },
                    header: text(q, "header").into(),
                    question: text(q, "question").into(),
                    options: q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| (text(o, "label").into(), text(o, "description").into()))
                        .collect(),
                    multiple: if opencode {
                        q["multiple"].as_bool().unwrap_or(false)
                    } else {
                        claude && q["multiSelect"].as_bool().unwrap_or(false)
                    },
                    custom: if opencode {
                        q["custom"].as_bool().unwrap_or(true)
                    } else {
                        claude
                            || q["isOther"].as_bool().unwrap_or(false)
                            || q["options"].as_array().is_none_or(Vec::is_empty)
                    },
                    secret: q["isSecret"].as_bool().unwrap_or(false),
                }
            })
            .collect()
    }
}

impl Protocol {
    pub(super) fn accept_user_input(
        &self,
        wire_id: &Value,
        input: Value,
        kind: ApprovalKind,
        step: &mut Step,
    ) {
        let approval = Approval {
            id: id_text(wire_id),
            wire_id: wire_id.clone(),
            input,
            title: conduit::i18n::text_static("user_input.the_cli_needs_you_to_confirm_options")
                .into(),
            detail: String::new(),
            allow_once: true,
            kind,
        };
        let questions = approval.questions();
        let mut ids = std::collections::HashSet::new();
        if !(wire_id.is_string() || wire_id.is_i64() || wire_id.is_u64())
            || wire_id.as_str() == Some("")
            || questions.is_empty()
            || questions.iter().any(|q| {
                q.id.trim().is_empty() || q.question.trim().is_empty() || !ids.insert(&q.id)
            })
        {
            step.done = true;
            step.error = Some(
                conduit::i18n::text_static(
                    "protocol.user_input.invalid_cli_question_request_stopped",
                )
                .into(),
            );
            return;
        }
        let mut approval = approval;
        approval.detail = questions
            .iter()
            .map(|q| q.question.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        step.events.push(Event::Approval(approval));
    }

    pub fn user_input_response(
        &self,
        approval: &Approval,
        answers: &BTreeMap<String, Vec<String>>,
    ) -> Option<Value> {
        let questions = approval.questions();
        if questions.is_empty()
            || questions.len() != answers.len()
            || questions.iter().any(|q| {
                answers.get(&q.id).is_none_or(|a| {
                    a.is_empty()
                        || (!q.multiple && a.len() != 1)
                        || a.iter().any(|s| {
                            s.trim().is_empty()
                                || (!q.custom && !q.options.iter().any(|(label, _)| label == s))
                        })
                })
            })
        {
            return None;
        }
        Some(match approval.kind {
            ApprovalKind::CodexUserInput => {
                let answers: BTreeMap<_, _> = answers
                    .iter()
                    .map(|(id, a)| (id, json!({"answers":a})))
                    .collect();
                json!({"id":approval.wire_id,"result":{"answers":answers}})
            }
            ApprovalKind::ClaudeUserInput => {
                let answers: BTreeMap<_, _> =
                    answers.iter().map(|(id, a)| (id, a.join(", "))).collect();
                let mut input = approval.input.clone();
                input["answers"] = json!(answers);
                json!({"type":"control_response","response":{"subtype":"success","request_id":approval.wire_id,
                    "response":{"behavior":"allow","updatedInput":input}}})
            }
            ApprovalKind::OpenCodeUserInput => {
                let answers: Vec<_> = questions.iter().map(|q| &answers[&q.id]).collect();
                json!({"answers":answers})
            }
            _ => return None,
        })
    }

    pub(super) fn user_input_cancel_response(&self, approval: &Approval) -> Value {
        match approval.kind {
            ApprovalKind::CodexUserInput => json!({"id":approval.wire_id,"result":{"answers":{}}}),
            ApprovalKind::OpenCodeUserInput => json!({"reject":true}),
            ApprovalKind::ClaudeUserInput => {
                json!({"type":"control_response","response":{"subtype":"success","request_id":approval.wire_id,
                "response":{"behavior":"deny","message":"用户跳过了本次问题"}}})
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(protocol: &mut Protocol) -> Approval {
        let step = if protocol.provider == Provider::Claude {
            protocol.accept(json!({"type":"control_request","request_id":"question-ticket","request":{
                "subtype":"can_use_tool","tool_name":"AskUserQuestion","input":{"questions":[
                    {"header":"方案","question":"选择方案？","multiSelect":true,"options":[
                        {"label":"方案 A","description":"第一个方案"},{"label":"方案 B","description":"第二个方案"}]},
                    {"header":"名称","question":"项目叫什么？","multiSelect":false,"options":[]}
                ]}}}))
        } else {
            protocol.accept(json!({"id":72,"method":"item/tool/requestUserInput","params":{"itemId":"question-tool","questions":[
                {"id":"choice","header":"方案","question":"选择方案？","isOther":false,"isSecret":false,"options":[
                    {"label":"方案 A","description":"第一个方案"},{"label":"方案 B","description":"第二个方案"}]},
                {"id":"name","header":"名称","question":"项目叫什么？","isOther":true,"isSecret":false,"options":[]}
            ]}}))
        };
        assert!(!step.done);
        assert!(
            step.writes.is_empty(),
            "a question must wait for explicit input"
        );
        let Event::Approval(approval) = step.events.into_iter().next().unwrap() else {
            panic!("question not queued");
        };
        approval
    }

    #[test]
    fn user_input_waits_in_every_permission_mode_and_uses_provider_answer_shapes() {
        let _locale = crate::test_support::chinese();
        for provider in [Provider::Codex, Provider::Claude] {
            for permission in [
                PermissionLevel::ReadOnly,
                PermissionLevel::WorkspaceWrite,
                PermissionLevel::FullAccess,
            ] {
                let mut protocol = Protocol::new(provider, "C:\\work".into(), "test".into(), None)
                    .with_permission(permission);
                let approval = request(&mut protocol);
                assert_eq!(protocol.automatic_approval(&approval), None);
                protocol.enable_full_access();
                assert_eq!(protocol.automatic_approval(&approval), None);
                let questions = approval.questions();
                assert_eq!(questions.len(), 2);
                assert_eq!(questions[0].options[1].1, "第二个方案");
                assert_eq!(questions[0].multiple, provider == Provider::Claude);
                let mut answers = BTreeMap::new();
                answers.insert(
                    questions[0].id.clone(),
                    if provider == Provider::Claude {
                        vec!["方案 A".into(), "方案 B".into()]
                    } else {
                        vec!["方案 B".into()]
                    },
                );
                assert!(
                    protocol.user_input_response(&approval, &answers).is_none(),
                    "incomplete answers must not consume the request"
                );
                answers.insert(questions[1].id.clone(), vec!["中文项目".into()]);
                let response = protocol.user_input_response(&approval, &answers).unwrap();
                if provider == Provider::Codex {
                    assert_eq!(
                        response,
                        json!({"id":72,"result":{"answers":{"choice":{"answers":["方案 B"]},"name":{"answers":["中文项目"]}}}})
                    );
                    assert_eq!(
                        protocol.approval_response(&approval, false),
                        json!({"id":72,"result":{"answers":{}}})
                    );
                    answers.get_mut("choice").unwrap().push("方案 A".into());
                    assert!(protocol.user_input_response(&approval, &answers).is_none());
                    answers.insert("choice".into(), vec!["没有提供的选项".into()]);
                    assert!(protocol.user_input_response(&approval, &answers).is_none());
                } else {
                    assert_eq!(response["response"]["request_id"], "question-ticket");
                    assert_eq!(response["response"]["response"]["behavior"], "allow");
                    let input = &response["response"]["response"]["updatedInput"];
                    assert_eq!(input["questions"], approval.input["questions"]);
                    assert_eq!(
                        input["answers"],
                        json!({"选择方案？":"方案 A, 方案 B","项目叫什么？":"中文项目"})
                    );
                    assert_eq!(
                        protocol.approval_response(&approval, false)["response"]["response"]["behavior"],
                        "deny"
                    );
                }
                let protocol = protocol.text_only();
                assert_eq!(protocol.automatic_approval(&approval), Some(false));
            }
        }
    }

    #[test]
    fn malformed_user_input_stops_instead_of_silently_skipping() {
        let _locale = crate::test_support::chinese();
        let mut protocol = Protocol::new(Provider::Codex, "C:\\work".into(), "test".into(), None);
        for params in [
            json!({"questions":[]}),
            json!({"questions":[{"id":"x","question":"?"},{"id":"x","question":"?"}]}),
            json!({"questions":[{"id":"x"}]}),
        ] {
            let step = protocol
                .accept(json!({"id":9,"method":"item/tool/requestUserInput","params":params}));
            assert!(step.done);
            assert!(step.error.is_some());
            assert!(step.writes.is_empty());
        }
    }
}
