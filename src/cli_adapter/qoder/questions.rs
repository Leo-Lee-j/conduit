//! Translate Qoder's ACP form elicitation into the existing question UI.
use super::*;
use std::collections::HashSet;

pub(super) fn approval(value: &Value) -> Result<Approval> {
    ensure!(valid_id(&value["id"]), "Invalid request ID");
    let params = &value["params"];
    ensure!(
        params["mode"].is_null() || params["mode"] == "form",
        "Only form elicitation is supported"
    );
    let schema = &params["requestedSchema"];
    ensure!(schema["type"] == "object", "Expected an object schema");
    let properties = schema["properties"]
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Missing properties"))?;
    ensure!(
        !properties.is_empty() && properties.len() <= 32,
        "Invalid question count"
    );
    let mut questions = Vec::new();
    for (id, property) in properties {
        ensure!(!id.trim().is_empty(), "Empty question ID");
        let multiple = property["type"] == "array";
        let field = if multiple {
            &property["items"]
        } else {
            property
        };
        let kind = field["type"].as_str().unwrap_or("string");
        ensure!(
            matches!(kind, "string" | "boolean" | "integer" | "number"),
            "Unsupported field type"
        );
        ensure!(!multiple || kind == "string", "Unsupported array field");
        // The question UI supports primitive fields and finite choices, not a
        // general JSON Schema validator. Reject constraints we cannot enforce.
        ensure!(
            ![
                "pattern",
                "allOf",
                "not",
                "$ref",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "multipleOf"
            ]
            .iter()
            .any(|key| field.get(key).is_some())
                && field["format"].as_str().is_none_or(|v| v == "password"),
            "Unsupported field constraint"
        );
        let choices = choices(field)?;
        ensure!(!multiple || !choices.is_empty(), "Expected array choices");
        let options: Vec<_> = choices
            .iter()
            .map(|(label, _)| json!({"label":label,"description":""}))
            .collect();
        let title = property["title"].as_str().unwrap_or(id);
        let question = property["description"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(title);
        ensure!(!question.trim().is_empty(), "Empty question");
        questions.push(json!({"id":id,"header":title,"question":question,"options":options,
            "multiple":multiple,"custom":choices.is_empty(),"isSecret":property["format"]=="password"}));
    }
    Ok(Approval {
        id: format!("qoder-input:{}", value["id"]),
        wire_id: value["id"].clone(),
        title: params["message"].as_str().unwrap_or("Qoder").into(),
        detail: questions
            .iter()
            .filter_map(|q| q["question"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        input: json!({"questions":questions,"schema":schema}),
        allow_once: true,
        kind: ApprovalKind::QoderUserInput,
    })
}

fn choices(field: &Value) -> Result<Vec<(String, Value)>> {
    let options = if field["type"] == "boolean" {
        vec![("true".into(), json!(true)), ("false".into(), json!(false))]
    } else if let Some(values) = field["enum"].as_array() {
        values
            .iter()
            .enumerate()
            .map(|(index, v)| {
                let raw = v
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Expected string enum"))?;
                let label = field["enumNames"][index].as_str().unwrap_or(raw);
                Ok((label.to_string(), v.clone()))
            })
            .collect::<Result<Vec<_>>>()?
    } else if let Some(values) = field["oneOf"]
        .as_array()
        .or_else(|| field["anyOf"].as_array())
    {
        values
            .iter()
            .map(|v| {
                let raw = v["const"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Expected string choice"))?;
                Ok((v["title"].as_str().unwrap_or(raw).into(), json!(raw)))
            })
            .collect::<Result<Vec<_>>>()?
    } else {
        vec![]
    };
    let mut labels = HashSet::new();
    ensure!(
        options.len() <= 128
            && options
                .iter()
                .all(|(label, _)| !label.trim().is_empty() && labels.insert(label)),
        "Ambiguous choices"
    );
    Ok(options)
}

pub(super) fn response(
    approval: &Approval,
    answers: &BTreeMap<String, Vec<String>>,
) -> Option<Value> {
    if approval.kind != ApprovalKind::QoderUserInput {
        return None;
    }
    let properties = approval.input["schema"]["properties"].as_object()?;
    if answers.len() != properties.len() {
        return None;
    }
    let mut content = serde_json::Map::new();
    for (id, property) in properties {
        let answer = answers.get(id)?;
        let multiple = property["type"] == "array";
        if answer.is_empty() || (!multiple && answer.len() != 1) {
            return None;
        }
        let field = if multiple {
            &property["items"]
        } else {
            property
        };
        let options = choices(field).ok()?;
        let values: Vec<Value> = answer
            .iter()
            .map(|text| {
                if text.trim().is_empty() {
                    return None;
                }
                let value = if !options.is_empty() {
                    options.iter().find(|(label, _)| label == text)?.1.clone()
                } else {
                    match field["type"].as_str().unwrap_or("string") {
                        "integer" => json!(text.parse::<i64>().ok()?),
                        "number" => {
                            Value::Number(serde_json::Number::from_f64(text.parse().ok()?)?)
                        }
                        "string" => json!(text),
                        _ => return None,
                    }
                };
                if let Some(number) = value.as_f64()
                    && (field["minimum"].as_f64().is_some_and(|min| number < min)
                        || field["maximum"].as_f64().is_some_and(|max| number > max))
                {
                    return None;
                }
                if let Some(s) = value.as_str() {
                    let length = s.chars().count() as u64;
                    if field["minLength"].as_u64().is_some_and(|min| length < min)
                        || field["maxLength"].as_u64().is_some_and(|max| length > max)
                    {
                        return None;
                    }
                }
                Some(value)
            })
            .collect::<Option<_>>()?;
        let value = if multiple {
            let count = values.len() as u64;
            if property["minItems"].as_u64().is_some_and(|min| count < min)
                || property["maxItems"].as_u64().is_some_and(|max| count > max)
            {
                return None;
            }
            let mut unique = HashSet::new();
            if !values.iter().all(|v| unique.insert(v.to_string())) {
                return None;
            }
            json!(values)
        } else {
            values.into_iter().next()?
        };
        content.insert(id.clone(), value);
    }
    Some(
        json!({"jsonrpc":"2.0","id":approval.wire_id,"result":{"action":"accept","content":content}}),
    )
}
