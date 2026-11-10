//! OpenCode 1.19.2 ACP advertises prompt commands (including user-defined ones).
//! Its native argument parser accepts a tail even when `input` is omitted.
use super::*;
use crate::commands::{
    Arguments, CommandId, Descriptor, Execution, MAX_BYTES, MAX_COMMANDS, RequestIntent,
};

fn catalog(value: &Value) -> Result<Vec<Descriptor>, String> {
    let invalid = || conduit::i18n::text("commands.invalid_catalog");
    let values = value.as_array().ok_or_else(invalid)?;
    if values.len() > MAX_COMMANDS {
        return Err(conduit::i18n::text("commands.catalog_limit"));
    }
    let mut items = Vec::with_capacity(values.len());
    let mut bytes = 0;
    let mut names = std::collections::HashSet::new();
    for value in values {
        let name = value["name"]
            .as_str()
            .filter(|name| crate::commands::valid_name(name))
            .ok_or_else(invalid)?;
        if !names.insert(name) {
            return Err(invalid());
        }
        let item = Descriptor {
            revision: 0,
            unavailable: None,
            id: CommandId {
                source: crate::cli_adapter::OPENCODE_COMMANDS.id.into(),
                local: name.into(),
            },
            name: name.into(),
            aliases: vec![],
            summary: value["description"].as_str().unwrap_or_default().into(),
            arguments: Arguments::TailText {
                required: false,
                hint: value["input"]["hint"].as_str().unwrap_or_default().into(),
            },
            execution: Execution::Engine,
        };
        bytes += item.bytes();
        if bytes > MAX_BYTES {
            return Err(conduit::i18n::text("commands.catalog_limit"));
        }
        items.push(item);
    }
    Ok(items)
}

impl Protocol {
    pub fn set_command_metadata(&mut self, metadata: Result<Value, String>) {
        self.acp.command_metadata = Some(metadata);
    }
    pub fn check_command_timeout(&self) -> Result<()> {
        if self.acp.stage == Stage::Commands
            && self
                .acp
                .commands_wait
                .is_some_and(|since| since.elapsed() > std::time::Duration::from_secs(8))
        {
            bail!(conduit::i18n::text("commands.no_catalog"));
        }
        Ok(())
    }

    pub(super) fn acp_commands_update(&mut self, params: &Value, step: &mut Step) {
        let Some(id) = params["sessionId"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 1024)
        else {
            return;
        };
        if self
            .acp
            .session_id
            .as_deref()
            .is_some_and(|expected| expected != id)
        {
            return;
        }
        let commands =
            catalog(&params["update"]["availableCommands"]).and_then(|items| {
                match self.acp.command_metadata.take() {
                    Some(Ok(metadata)) => verify_metadata(items, &metadata),
                    Some(Err(error)) => Err(error),
                    None => Err(conduit::i18n::text("commands.no_metadata")),
                }
            });
        // New-session notifications can precede the response. Stage them until
        // the actual session ID has been confirmed; never publish unbound data.
        if self.acp.session_id.is_none() && self.acp.stage == Stage::Session {
            self.acp.pending_commands = Some((id.into(), commands));
            return;
        }
        if self.acp.session_id.as_deref() != Some(id) {
            return;
        }
        self.acp.commands = Some(commands.clone());
        step.events.push(Event::CommandCatalog(commands));
        if self.acp.stage == Stage::Commands {
            self.acp_command_ready(step);
        }
    }

    pub(super) fn acp_command_ready(&mut self, step: &mut Step) {
        let Some(commands) = &self.acp.commands else {
            return;
        };
        let error = match commands {
            Err(error) => Some(error.clone()),
            Ok(commands) => match &self.intent {
                RequestIntent::Command(call) if call.text().contains(['!', '`', '$']) => {
                    Some(conduit::i18n::text("commands.unsafe_arguments"))
                }
                RequestIntent::Command(call)
                    if !call.verify(
                        crate::cli_adapter::OPENCODE_COMMANDS,
                        self.acp.session_id.as_deref(),
                        commands,
                    ) || call.text() != self.prompt =>
                {
                    Some(conduit::i18n::text("commands.stale"))
                }
                _ => None,
            },
        };
        if error.is_some() || matches!(self.intent, RequestIntent::Discover) {
            self.acp.stage = Stage::Done;
            step.done = true;
            step.error = error;
        } else {
            self.acp_send_prompt(step);
        }
    }
}

/// ACP's short list omits agent/model overrides, subtasks and shell expansion.
/// The authenticated native metadata endpoint is required before execution.
fn verify_metadata(
    mut items: Vec<Descriptor>,
    metadata: &Value,
) -> Result<Vec<Descriptor>, String> {
    use std::hash::{Hash, Hasher};
    let values = metadata
        .as_array()
        .filter(|items| items.len() <= MAX_COMMANDS)
        .ok_or_else(|| conduit::i18n::text("commands.no_metadata"))?;
    for item in &mut items {
        let matches: Vec<_> = values
            .iter()
            .filter(|entry| entry["name"].as_str() == Some(&item.name))
            .collect();
        let safe = matches.len() == 1 && {
            let value = matches[0];
            value["source"] == "command"
                && value["agent"].is_null()
                && value["model"].is_null()
                && value["subtask"] != true
                && value["template"]
                    .as_str()
                    .is_some_and(|template| !template.contains("!`"))
        };
        if !safe {
            item.unavailable = Some(conduit::i18n::text("commands.unsupported_effects"));
        }
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for entry in matches {
            entry.to_string().hash(&mut hash);
        }
        item.revision = hash.finish();
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Catalog, CommandContext};

    fn update(id: &str, items: Value) -> Value {
        json!({"method":"session/update","params":{"sessionId":id,"update":{"sessionUpdate":"available_commands_update","availableCommands":items}}})
    }

    fn metadata(items: &Value) -> Value {
        json!(
            items
                .as_array()
                .unwrap()
                .iter()
                .map(|item| json!({
                    "name":item["name"], "source":"command", "template":"Review $ARGUMENTS"
                }))
                .collect::<Vec<_>>()
        )
    }

    fn receive(protocol: &mut Protocol, id: &str, items: Value) -> Step {
        protocol.set_command_metadata(Ok(metadata(&items)));
        protocol.accept(update(id, items))
    }

    fn init(protocol: &mut Protocol) -> Step {
        protocol.accept(json!({"id":1,"result":{"protocolVersion":1,"agentInfo":{"name":"OpenCode"},"agentCapabilities":{"loadSession":true}}}))
    }

    #[test]
    fn discovery_never_prompts_and_binds_early_or_late_catalogs_to_confirmed_session() {
        for early in [true, false] {
            let mut protocol = Protocol::new(
                Provider::OpenCode,
                "C:/fixture".into(),
                "must not send".into(),
                None,
            )
            .with_intent(RequestIntent::Discover);
            let request = init(&mut protocol);
            assert_eq!(request.writes[0]["method"], "session/new");
            if early {
                let step = receive(
                    &mut protocol,
                    "native",
                    json!([{ "name":"custom:review", "description":"Review" }]),
                );
                assert!(step.events.is_empty() && step.writes.is_empty());
            }
            let response = protocol.accept(json!({"id":2,"result":{"sessionId":"native"}}));
            assert!(response.writes.is_empty());
            assert_eq!(response.done, early);
            if !early {
                assert!(!receive(&mut protocol, "foreign", json!([])).done);
                let done = receive(&mut protocol, "native", json!([]));
                assert!(done.done && done.error.is_none() && done.writes.is_empty());
                assert!(
                    matches!(&done.events[0], Event::CommandCatalog(Ok(items)) if items.is_empty())
                );
            }
        }
    }

    #[test]
    fn explicit_command_waits_for_fresh_metadata_and_refuses_removed_commands() {
        let items =
            json!([{ "name":"custom:review", "description":"Review", "input":{"hint":"target"} }]);
        let descriptor = verify_metadata(catalog(&items).unwrap(), &metadata(&items))
            .unwrap()
            .remove(0);
        let context = CommandContext {
            workspace: "C:/fixture".into(),
            session: Some(1),
            native_id: Some("native".into()),
            source: "opencode".into(),
            registration: 1,
            configuration: 1,
        };
        let mut cache = Catalog::default();
        let ticket = cache.begin(context.clone());
        cache.publish(&ticket, Ok(vec![descriptor.clone()]));
        let text = "/custom:review  中文 \"a b\"";
        for removed in [false, true] {
            let call = cache.prepare(&context, &descriptor, text.into()).unwrap();
            let mut protocol = Protocol::new(
                Provider::OpenCode,
                "C:/fixture".into(),
                text.into(),
                Some("native".into()),
            )
            .with_intent(RequestIntent::Command(Box::new(call)));
            protocol.acp.session_id = Some("native".into());
            protocol.acp.stage = Stage::Configure;
            let mut waiting = Step::new();
            protocol.acp_next(&mut waiting);
            assert!(waiting.writes.is_empty() && !waiting.done);
            let step = receive(
                &mut protocol,
                "native",
                if removed { json!([]) } else { items.clone() },
            );
            if removed {
                assert!(step.done && step.error.is_some() && step.writes.is_empty());
            } else {
                assert_eq!(step.writes[0]["method"], "session/prompt");
                assert_eq!(
                    step.writes[0]["params"]["prompt"],
                    json!([{"type":"text","text":text}])
                );
                let done = protocol
                    .accept(json!({"id":step.writes[0]["id"],"result":{"stopReason":"end_turn"}}));
                assert!(done.done && done.error.is_none());
                assert!(
                    protocol
                        .accept(update("native", json!([])))
                        .events
                        .is_empty()
                );
            }
        }
    }

    #[test]
    fn metadata_is_bounded_and_catalog_updates_are_full_replacements() {
        assert!(catalog(&json!([{ "name":"x/y" }])).is_err());
        assert!(catalog(&json!([{ "name":"x" },{ "name":"x" }])).is_err());
        assert!(catalog(&json!([{ "name":"x", "description":"x".repeat(MAX_BYTES) }])).is_err());
        let mut protocol = Protocol::new(
            Provider::OpenCode,
            "C:/fixture".into(),
            "message".into(),
            Some("native".into()),
        );
        init(&mut protocol);
        let ready = receive(&mut protocol, "native", json!([{ "name":"x" }]));
        assert!(matches!(&ready.events[0], Event::CommandCatalog(Ok(items)) if items.len() == 1));
        let cleared = receive(&mut protocol, "native", json!([]));
        assert!(matches!(&cleared.events[0], Event::CommandCatalog(Ok(items)) if items.is_empty()));
        protocol.acp.stage = Stage::Commands;
        protocol.acp.commands_wait =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(9));
        assert!(protocol.check_command_timeout().is_err());
    }

    #[test]
    fn command_effects_and_changed_templates_cannot_bypass_managed_permissions() {
        let advertised = json!([{ "name":"test" }]);
        let safe = json!([{ "name":"test", "source":"command", "template":"Review $ARGUMENTS" }]);
        let original = verify_metadata(catalog(&advertised).unwrap(), &safe)
            .unwrap()
            .remove(0);
        assert!(original.unavailable.is_none());
        for (key, value) in [
            ("agent", json!("unrestricted")),
            ("model", json!("other/model")),
            ("subtask", json!(true)),
            ("source", json!("mcp")),
            ("template", json!({})),
            ("template", json!("Run !`echo unsafe`")),
        ] {
            let mut unsafe_metadata = safe.clone();
            unsafe_metadata[0][key] = value;
            let item = verify_metadata(catalog(&advertised).unwrap(), &unsafe_metadata)
                .unwrap()
                .remove(0);
            assert!(item.unavailable.is_some(), "{key}");
        }
        let mut changed = safe.clone();
        changed[0]["template"] = json!("Changed task $ARGUMENTS");
        let item = verify_metadata(catalog(&advertised).unwrap(), &changed)
            .unwrap()
            .remove(0);
        assert_ne!(original.revision, item.revision);
        let context = CommandContext {
            workspace: "C:/fixture".into(),
            session: Some(1),
            native_id: Some("native".into()),
            source: "opencode".into(),
            registration: 1,
            configuration: 1,
        };
        let mut cache = Catalog::default();
        let ticket = cache.begin(context.clone());
        cache.publish(&ticket, Ok(vec![original.clone()]));
        let text = "/test !`echo unsafe`";
        let call = cache.prepare(&context, &original, text.into()).unwrap();
        let mut protocol = Protocol::new(
            Provider::OpenCode,
            "C:/fixture".into(),
            text.into(),
            Some("native".into()),
        )
        .with_intent(RequestIntent::Command(Box::new(call)));
        protocol.acp.stage = Stage::Commands;
        protocol.acp.session_id = Some("native".into());
        protocol.set_command_metadata(Ok(safe));
        let blocked = protocol.accept(update("native", advertised));
        assert!(blocked.done && blocked.error.is_some() && blocked.writes.is_empty());
    }
}
