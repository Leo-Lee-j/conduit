//! Codex question history is independent of the ordinary tool/message fallback.
use super::*;
use crate::domain::{ChatItemData, QuestionRecord, ToolApproval};
use crate::message_status::{ApprovalStatus, ToolStatus};

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct State {
    requests: HashMap<String, (usize, ChatItem)>,
}

impl State {
    /// Consume only recognized question records; unrelated messages keep their normal path.
    pub(super) fn observe(&mut self, index: usize, created_at: u64, value: &Value) -> bool {
        let payload = &value["payload"];
        match (value["type"].as_str(), payload["type"].as_str()) {
            (Some("event_msg"), Some("item_completed")) => {
                let item = &payload["item"];
                if item["type"] == "AgentMessage" && item["delivery"] == "async" {
                    return self.record(index, created_at, item, true);
                }
                if item["type"] == "UserMessage" {
                    return super::codex_event_text(item).is_some_and(|text| self.reply(&text));
                }
            }
            (Some("response_item"), Some("function_call")) => {
                if matches!(
                    payload["name"].as_str(),
                    Some("request_user_input" | "request_user_input_async")
                ) {
                    return self.record(index, created_at, payload, false);
                }
            }
            (Some("response_item"), Some("function_call_output")) => {
                if let Some((_, item)) = payload["call_id"]
                    .as_str()
                    .and_then(|id| self.requests.get_mut(id))
                    && let Some(output) = payload["output"]
                        .as_str()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                    && let Some(answers) = output["answers"].as_object()
                {
                    let approval = item.approval.as_mut().unwrap();
                    let mut answered = false;
                    for question in &mut approval.questions {
                        let value = &answers.get(&question.id).unwrap_or(&Value::Null)["answers"];
                        answered |= answer(question, value);
                    }
                    approval.status = if answered {
                        ApprovalStatus::Answered
                    } else {
                        ApprovalStatus::Skipped
                    };
                    return true;
                }
            }
            (Some("response_item"), Some("message")) if payload["role"] == "user" => {
                // Only consume a whole transport reply, never a mixed user message.
                if let Some(parts) = payload["content"].as_array()
                    && parts.len() == 1
                    && let Some(text) = parts[0]["text"].as_str()
                {
                    return self.reply(text);
                }
            }
            _ => {}
        }
        false
    }

    fn record(&mut self, index: usize, created_at: u64, value: &Value, event: bool) -> bool {
        let Some(request) = value[if event { "id" } else { "call_id" }]
            .as_str()
            .filter(|id| !id.is_empty())
        else {
            return false;
        };
        let input = if event {
            value.clone()
        } else {
            let Some(input) = value["arguments"]
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
            else {
                return false;
            };
            input
        };
        let Some(questions) = records(&input["questions"]) else {
            return false;
        };
        let key = if event {
            format!("import-{index}-{request}")
        } else {
            format!("import-{index}-call")
        };
        if let Some((position, item)) = self.requests.get_mut(request) {
            // Native async questions can have both a function call and an AgentMessage.
            // Prefer the event's key so an older imported text bubble can be upgraded.
            if event && item.key.ends_with("-call") {
                *position = index;
                item.key = key;
            }
            return true;
        }
        self.requests.insert(
            request.into(),
            (
                index,
                ChatItemData {
                    key,
                    created_at,
                    kind: ItemKind::Tool,
                    text: conduit::i18n::text("user_input.the_cli_needs_you_to_confirm_options"),
                    input: String::new(),
                    detail: String::new(),
                    status: ToolStatus::Completed,
                    approval: Some(ToolApproval {
                        request: request.into(),
                        status: ApprovalStatus::Interrupted,
                        questions,
                    }),
                    model: None,
                    reasoning_summary: None,
                    elapsed_ms: None,
                }
                .into(),
            ),
        );
        true
    }

    fn reply(&mut self, text: &str) -> bool {
        if let Some(body) = text
            .trim()
            .strip_prefix("<send_user_message_question_reply>")
            .and_then(|body| body.strip_suffix("</send_user_message_question_reply>"))
            && let Ok(Value::Array(replies)) = serde_json::from_str::<Value>(body.trim())
        {
            let total = replies.len();
            let mut matched = 0;
            for reply in replies {
                let Some(id) = reply["questionItemId"].as_str() else {
                    continue;
                };
                let identity = serde_json::from_str::<Value>(id)
                    .or_else(|_| serde_json::from_str(&id.replace("\\\"", "\"")));
                let Ok(Value::Array(identity)) = identity else {
                    continue;
                };
                if identity.len() != 3 || identity[0] != "request_user_input_async" {
                    continue;
                }
                let Some((_, item)) = identity[1]
                    .as_str()
                    .and_then(|id| self.requests.get_mut(id))
                else {
                    continue;
                };
                let approval = item.approval.as_mut().unwrap();
                let Some(question) = identity[2]
                    .as_u64()
                    .and_then(|index| approval.questions.get_mut(index as usize))
                else {
                    continue;
                };
                let Some(text) = reply["answer"].as_str() else {
                    continue;
                };
                answer(question, &serde_json::json!([text]));
                approval.status = ApprovalStatus::Answered;
                matched += 1;
            }
            return matched > 0 && matched == total;
        }
        const PREFIX: &str = "Conduit paused execution to wait for the user's response to your questions. Here are the submitted responses (skipped questions have no answer):\n";
        let Some(body) = text.strip_prefix(PREFIX) else {
            return false;
        };
        let Some(Ok(Value::Array(replies))) = serde_json::Deserializer::from_str(body)
            .into_iter::<Value>()
            .next()
        else {
            return false;
        };
        let total = replies.len();
        let mut matched = 0;
        for reply in replies {
            let Some((_, item)) = reply["requestId"]
                .as_str()
                .and_then(|id| self.requests.get_mut(id))
            else {
                continue;
            };
            let approval = item.approval.as_mut().unwrap();
            let Some(replies) = reply["questions"]
                .as_array()
                .filter(|replies| replies.len() == approval.questions.len())
            else {
                continue;
            };
            if !replies
                .iter()
                .zip(&approval.questions)
                .all(|(reply, question)| {
                    reply["question"].as_str() == Some(question.question.as_str())
                })
            {
                continue;
            }
            let mut answered = false;
            for (reply, question) in replies.iter().zip(&mut approval.questions) {
                if reply["skipped"] != true {
                    answered |= answer(question, &reply["answers"]);
                }
            }
            approval.status = if answered {
                ApprovalStatus::Answered
            } else {
                ApprovalStatus::Skipped
            };
            matched += 1;
        }
        matched > 0 && matched == total
    }

    pub(super) fn rows(self) -> impl Iterator<Item = (usize, usize, ChatItem)> {
        self.requests
            .into_values()
            .map(|(index, item)| (index, 0, item))
    }
}

fn records(value: &Value) -> Option<Vec<QuestionRecord>> {
    let values = value.as_array().filter(|questions| !questions.is_empty())?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let question = value["question"]
                .as_str()
                .or_else(|| value["title"].as_str())
                .filter(|text| !text.trim().is_empty())?;
            let options = value["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|option| {
                    Some((
                        option
                            .as_str()
                            .or_else(|| option["label"].as_str())?
                            .to_owned(),
                        option["description"].as_str().unwrap_or_default().into(),
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            Some(QuestionRecord {
                id: value["id"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("question_{}", index + 1)),
                header: value["header"].as_str().unwrap_or_default().into(),
                question: question.into(),
                options,
                multiple: value["multiple"].as_bool().unwrap_or(false),
                secret: value["isSecret"].as_bool().unwrap_or(false),
                selected: vec![],
                answers: vec![],
            })
        })
        .collect()
}

fn answer(question: &mut QuestionRecord, value: &Value) -> bool {
    let Some(answers) = value.as_array().and_then(|values| {
        values
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
    }) else {
        return false;
    };
    let answered = !answers.is_empty();
    if !question.secret {
        question.selected = question
            .options
            .iter()
            .enumerate()
            .filter_map(|(index, (label, _))| answers.contains(label).then_some(index))
            .collect();
        question.answers = answers;
    }
    answered
}

/// Upgrade matching native items; locally acknowledged cards remain authoritative.
pub(super) fn restore_imported(previous: &mut Vec<ChatItem>, incoming: &[ChatItem]) {
    let questions: HashMap<_, _> = incoming
        .iter()
        .filter(|item| {
            item.approval
                .as_ref()
                .is_some_and(|approval| !approval.questions.is_empty())
        })
        .map(|item| (item.key.as_str(), item))
        .collect();
    if questions.is_empty() {
        return;
    }
    let native: HashMap<_, _> = questions
        .values()
        .map(|item| (item.approval.as_ref().unwrap().request.as_str(), *item))
        .collect();
    let mut replies = State::default();
    for (index, question) in incoming.iter().enumerate() {
        if let Some(approval) = &question.approval
            && !approval.questions.is_empty()
        {
            replies
                .requests
                .insert(approval.request.clone(), (index, question.clone()));
        }
    }
    previous.retain(|item| {
        !(item.key.starts_with("import-")
            && item.kind == ItemKind::User
            && replies.reply(&item.text))
    });
    for item in previous.iter_mut() {
        let question = questions.get(item.key.as_str()).copied().or_else(|| {
            // Earlier live Codex sessions stored async questions as assistant text.
            // The local turn prefix differs, but the native item ID remains intact.
            let native_id = item.key.strip_prefix("turn-")?.split_once('-')?.1;
            native.get(native_id).copied()
        });
        if let Some(question) = question
            && item.approval.as_ref().is_none_or(|approval| {
                approval.request == question.approval.as_ref().unwrap().request
                    && approval.status == ApprovalStatus::Interrupted
            })
        {
            let key = item.key.clone();
            *item = question.clone();
            item.key = key;
        }
    }
    // Older imports dropped request_user_input when other structured tools existed.
    // Reinsert those cards at a shared native key, without guessing positions in local turns.
    if !previous.is_empty() && previous.iter().all(|item| item.key.starts_with("import-")) {
        for (index, question) in incoming
            .iter()
            .enumerate()
            .filter(|(_, item)| questions.contains_key(item.key.as_str()))
        {
            if previous.iter().any(|item| item.key == question.key) {
                continue;
            }
            let next = incoming[index + 1..]
                .iter()
                .find_map(|next| previous.iter().position(|item| item.key == next.key));
            let position = next.or_else(|| {
                incoming[..index]
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(source, before)| {
                        let position = previous.iter().position(|item| item.key == before.key)?;
                        // A new user turn is appended by merge_transcript as a whole.
                        // Inserting its question here would duplicate it ahead of that turn.
                        (!incoming[source + 1..index]
                            .iter()
                            .any(|item| item.kind == ItemKind::User))
                        .then_some(position + 1)
                    })
            });
            if let Some(position) = position {
                previous.insert(position, question.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests;
