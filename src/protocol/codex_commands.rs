//! App-server operations are RPCs, never slash text sent to the model.
use super::*;
use crate::commands::{Arguments, CommandId, Descriptor, Execution, RequestIntent, Source};

pub const SOURCE: Source = Source {
    id: "codex",
    registration: 1,
};

fn catalog() -> Vec<Descriptor> {
    [
        ("compact", "commands.codex_compact"),
        ("review", "commands.codex_review"),
    ]
    .into_iter()
    .map(|(name, summary)| Descriptor {
        revision: 1,
        unavailable: None,
        id: CommandId {
            source: SOURCE.id.into(),
            local: name.into(),
        },
        name: name.into(),
        aliases: vec![],
        summary: conduit::i18n::text(summary),
        arguments: Arguments::None,
        execution: Execution::Engine,
    })
    .collect()
}

impl Protocol {
    pub(super) fn codex_command_ready(&mut self, step: &mut Step) -> bool {
        let commands = catalog();
        step.events
            .push(Event::CommandCatalog(Ok(commands.clone())));
        match &self.intent {
            RequestIntent::Message | RequestIntent::Fork(_) => false,
            RequestIntent::Discover => {
                step.done = true;
                true
            }
            RequestIntent::Command(call) => {
                if !call.verify(SOURCE, self.thread_id.as_deref(), &commands)
                    || call.text() != self.prompt
                {
                    step.done = true;
                    step.error = Some(conduit::i18n::text("commands.stale"));
                    return true;
                }
                let name = crate::commands::parse(call.text()).unwrap().name;
                let (method, params) = match name.as_str() {
                    "compact" => ("thread/compact/start", json!({"threadId":self.thread_id})),
                    "review" => (
                        "review/start",
                        json!({"threadId":self.thread_id,
                        "delivery":"inline", "target":{"type":"uncommittedChanges"}}),
                    ),
                    _ => {
                        step.done = true;
                        step.error = Some(conduit::i18n::text("commands.unknown"));
                        return true;
                    }
                };
                step.writes
                    .push(json!({"id":3,"method":method,"params":params}));
                true
            }
        }
    }

    pub(super) fn codex_compacting(&self) -> bool {
        matches!(&self.intent, RequestIntent::Command(call)
            if crate::commands::parse(call.text()).is_some_and(|q| q.name == "compact"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Catalog, CommandContext};
    fn command(name: &str) -> Protocol {
        let context = CommandContext {
            workspace: "C:/repo".into(),
            session: Some(1),
            native_id: Some("thread".into()),
            source: SOURCE.id.into(),
            registration: SOURCE.registration,
            configuration: 1,
        };
        let mut cache = Catalog::default();
        let ticket = cache.begin(context.clone());
        let items = catalog();
        let item = items.iter().find(|c| c.name == name).unwrap().clone();
        cache.publish(&ticket, Ok(items));
        let text = format!("/{name}");
        let call = cache.prepare(&context, &item, text.clone()).unwrap();
        Protocol::new(
            Provider::Codex,
            "C:/repo".into(),
            text,
            Some("thread".into()),
        )
        .with_intent(RequestIntent::Command(Box::new(call)))
    }
    #[test]
    fn native_discovery_sends_no_prompt_and_commands_use_explicit_rpcs() {
        let mut discover =
            Protocol::new(Provider::Codex, "C:/repo".into(), "never send".into(), None)
                .with_intent(RequestIntent::Discover);
        let step = discover.accept(json!({"id":2,"result":{"thread":{"id":"thread"}}}));
        assert!(step.done && step.writes.is_empty());
        assert!(
            step.events
                .iter()
                .any(|e| matches!(e, Event::CommandCatalog(Ok(v)) if v.len()==2))
        );
        for (name, method) in [
            ("review", "review/start"),
            ("compact", "thread/compact/start"),
        ] {
            let mut protocol = command(name);
            let step = protocol.accept(json!({"id":2,"result":{"thread":{"id":"thread"}}}));
            assert_eq!(step.writes[0]["method"], method);
            assert!(step.writes[0]["params"].get("input").is_none());
        }
    }
    #[test]
    fn compact_ack_is_not_completion_and_other_thread_items_are_ignored() {
        let mut protocol = command("compact");
        protocol.accept(json!({"id":2,"result":{"thread":{"id":"thread"}}}));
        assert!(!protocol.accept(json!({"id":3,"result":{}})).done);
        assert!(!protocol.accept(json!({"method":"item/completed","params":{"threadId":"other","item":{"type":"contextCompaction"}}})).done);
        assert!(!protocol.accept(json!({"method":"turn/completed","params":{"threadId":"thread","turn":{"id":"old","status":"completed"}}})).done);
        assert!(protocol.accept(json!({"method":"item/completed","params":{"threadId":"thread","item":{"type":"contextCompaction"}}})).done);
    }
}
