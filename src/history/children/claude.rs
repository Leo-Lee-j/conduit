//! Claude SDK-compatible subagent transcript layout and parentUuid chain.
use super::*;
use crate::domain::{ChatItem, ChatItemData, ItemKind};
use crate::message_status::ToolStatus;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read},
};

pub(super) fn read(cwd: &Path, binding: &ChildBinding, budget: &Budget<'_>) -> Result<Snapshot> {
    ensure!(
        safe_id(&binding.root_native) && safe_id(&binding.native),
        "Invalid Claude subagent identity"
    );
    let projects = contained(&binding.source.join("projects"), &binding.source)?;
    let mut matches = Vec::new();
    for (count, entry) in std::fs::read_dir(&projects)?.enumerate() {
        budget.check(0, count)?;
        ensure!(count < 4096, "Too many Claude project directories");
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let parent = entry.path().join(format!("{}.jsonl", binding.root_native));
        if parent.is_file() {
            matches.push((
                contained(&parent, &projects)?,
                entry.path().join(&binding.root_native).join("subagents"),
            ));
        }
    }
    ensure!(
        matches.len() == 1,
        "Claude parent history is missing or ambiguous"
    );
    let (parent, agents) = matches.pop().unwrap();
    let mut read_bytes = 0;
    let mut read_items = 0;
    let parent_entries = entries(&parent, budget, &mut read_bytes, &mut read_items)?;
    ensure!(
        parent_entries
            .iter()
            .any(|v| v["sessionId"] == binding.root_native
                && v["cwd"].as_str().is_some_and(|dir| same_dir(dir, cwd))),
        "Claude parent history workspace does not match"
    );
    drop(parent_entries);
    let agents = contained(&agents, &projects)?;
    let mut queue = vec![agents.clone()];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    let expected = format!("agent-{}.jsonl", binding.native);
    let mut count = 0;
    while let Some(dir) = queue.pop() {
        let dir = contained(&dir, &agents)?;
        ensure!(
            visited.insert(dir.clone()),
            "Repeated Claude subagent directory"
        );
        for entry in std::fs::read_dir(dir)? {
            count += 1;
            budget.check(0, count)?;
            ensure!(count <= 4096, "Too many Claude subagent files");
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                queue.push(entry.path());
            } else if entry.file_name() == expected.as_str() {
                files.push(contained(&entry.path(), &agents)?);
            }
        }
    }
    ensure!(
        files.len() == 1,
        "Claude subagent transcript is missing or ambiguous"
    );
    let path = files.pop().unwrap();
    // Nested task ancestry must be independently confirmed by the native sidecar.
    let sidecar = path.with_file_name(format!("agent-{}.meta.json", binding.native));
    if binding.parent != binding.root_native || sidecar.exists() {
        let sidecar = contained(&sidecar, &agents)?;
        let sidecar = File::open(sidecar)?;
        let metadata_size = sidecar.metadata()?.len();
        ensure!(
            metadata_size <= 1024 * 1024,
            "Claude metadata exceeds budget"
        );
        read_bytes += metadata_size;
        budget.check(read_bytes, read_items)?;
        let meta: Value = serde_json::from_reader(sidecar.take(metadata_size))?;
        if binding.parent != binding.root_native {
            ensure!(
                meta["parentAgentId"] == binding.parent,
                "Claude subagent parent does not match"
            );
        } else {
            ensure!(
                meta["parentAgentId"].is_null(),
                "Claude subagent belongs to another parent"
            );
        }
    }
    let entries = entries(&path, budget, &mut read_bytes, &mut read_items)?;
    ensure!(!entries.is_empty(), "Claude subagent transcript is empty");
    for entry in &entries {
        ensure!(
            entry
                .get("sessionId")
                .is_none_or(|id| id == &binding.root_native)
                && entry.get("agentId").is_none_or(|id| id == &binding.native)
                && entry["cwd"].as_str().is_none_or(|dir| same_dir(dir, cwd)),
            "Claude subagent transcript identity or workspace does not match"
        );
    }
    ensure!(
        entries
            .iter()
            .any(|v| v["agentId"] == binding.native && v["sessionId"] == binding.root_native),
        "Claude subagent transcript has no verified identity"
    );
    let chain = chain(&entries)?;
    ensure!(
        !chain.is_empty(),
        "Claude subagent transcript has no conversation"
    );
    Ok(Snapshot::Native {
        created_at: chain
            .first()
            .and_then(|v| v["timestamp"].as_str())
            .map(super::super::rfc3339_to_unix)
            .unwrap_or(0),
        messages: messages(&chain),
        // Claude agent IDs are not independently resumable session IDs.
        idle: false,
    })
}

fn entries(
    path: &Path,
    budget: &Budget<'_>,
    read_bytes: &mut u64,
    read_items: &mut usize,
) -> Result<Vec<Value>> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    *read_bytes += size;
    budget.check(*read_bytes, *read_items)?;
    let mut reader = BufReader::new(file.take(size));
    let mut result = Vec::new();
    loop {
        let mut bytes = Vec::new();
        let read = reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)?;
        if read == 0 {
            break;
        }
        ensure!(
            read <= 8 * 1024 * 1024,
            "Claude transcript line exceeds budget"
        );
        budget.check(*read_bytes, *read_items + 1)?;
        if bytes.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice(&bytes) {
            Ok(value) => {
                *read_items += 1;
                result.push(value);
            }
            // An in-flight final JSONL write is not yet a message.
            Err(_) if bytes.last() != Some(&b'\n') => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(result)
}

fn chain(entries: &[Value]) -> Result<Vec<&Value>> {
    let by_id: HashMap<_, _> = entries
        .iter()
        .filter_map(|v| v["uuid"].as_str().map(|id| (id, v)))
        .collect();
    let first = entries
        .iter()
        .find(|v| matches!(v["type"].as_str(), Some("user" | "assistant")));
    let mut current = entries
        .iter()
        .rev()
        .find(|v| matches!(v["type"].as_str(), Some("user" | "assistant")));
    let mut seen = HashSet::new();
    let mut chain = Vec::new();
    while let Some(entry) = current {
        let id = entry["uuid"]
            .as_str()
            .context("Missing Claude message identity")?;
        ensure!(seen.insert(id), "Cyclic Claude message history");
        chain.push(entry);
        current = match entry["parentUuid"].as_str() {
            Some(parent) => {
                let previous = by_id.get(parent).copied();
                ensure!(
                    previous.is_some() || first.is_some_and(|v| v["uuid"] == id),
                    "Incomplete Claude message history"
                );
                previous
            }
            None => None,
        };
    }
    chain.reverse();
    Ok(chain)
}

fn messages(chain: &[&Value]) -> Vec<ChatItem> {
    let mut messages: Vec<ChatItem> = Vec::new();
    let mut tools = HashMap::new();
    for entry in chain {
        if entry["isMeta"] == true {
            continue;
        }
        let user = entry["type"] == "user";
        if !user && entry["type"] != "assistant" {
            continue;
        }
        let content = &entry["message"]["content"];
        let parts = content.as_array().cloned().unwrap_or_else(|| {
            content
                .as_str()
                .map(|s| vec![serde_json::json!({"type":"text","text":s})])
                .unwrap_or_default()
        });
        for (index, part) in parts.iter().enumerate() {
            if part["type"] == "tool_result" {
                if let Some(item) = part["tool_use_id"]
                    .as_str()
                    .and_then(|id| tools.get(id))
                    .and_then(|i: &usize| messages.get_mut(*i))
                {
                    item.detail = readable(&part["content"]);
                    item.status = if part["is_error"] == true {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Completed
                    };
                }
                continue;
            }
            let kind = match part["type"].as_str() {
                Some("text") => {
                    if user {
                        ItemKind::User
                    } else {
                        ItemKind::Assistant
                    }
                }
                Some("thinking") if !user => ItemKind::Reasoning,
                Some("tool_use") if !user => ItemKind::Tool,
                Some("image" | "document") if user => ItemKind::User,
                _ => continue,
            };
            let text = match part["type"].as_str() {
                Some("thinking") => part["thinking"].as_str().unwrap_or_default(),
                Some("tool_use") => part["name"].as_str().unwrap_or("tool"),
                Some("image" | "document") => "@attachment",
                _ => part["text"].as_str().unwrap_or_default(),
            };
            if text.is_empty() {
                continue;
            }
            if kind == ItemKind::Tool
                && let Some(id) = part["id"].as_str()
            {
                tools.insert(id.to_owned(), messages.len());
            }
            messages.push(ChatItem::from(ChatItemData {
                key: format!(
                    "native-{}-{index}",
                    entry["uuid"].as_str().unwrap_or_default()
                ),
                kind: kind.clone(),
                text: text.into(),
                created_at: entry["timestamp"]
                    .as_str()
                    .map(super::super::rfc3339_to_unix)
                    .unwrap_or(0),
                input: if kind == ItemKind::Tool {
                    serde_json::to_string_pretty(&part["input"]).unwrap_or_default()
                } else {
                    String::new()
                },
                detail: String::new(),
                status: if kind == ItemKind::Tool {
                    ToolStatus::Interrupted
                } else {
                    Default::default()
                },
                approval: None,
                model: None,
                reasoning_summary: None,
                elapsed_ms: None,
            }));
        }
    }
    messages
}
fn readable(content: &Value) -> String {
    content.as_str().map(str::to_owned).unwrap_or_else(|| {
        content
            .as_array()
            .into_iter()
            .flatten()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}
