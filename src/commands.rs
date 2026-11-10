//! Chat command identities, bounded catalogs and explicit submission intent.
//! Independent of the composer, GPUI and any engine's wire protocol.
use std::{collections::VecDeque, ops::Range, path::PathBuf};

pub const MAX_COMMANDS: usize = 512;
pub const MAX_BYTES: usize = 512 * 1024;
const MAX_CONTEXTS: usize = 8;

#[derive(Clone, Copy)]
pub struct Source {
    pub id: &'static str,
    pub registration: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CommandId {
    pub source: String,
    pub local: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arguments {
    None,
    TailText { required: bool, hint: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostAction {
    NewChat,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Execution {
    Host(HostAction),
    Engine,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// Source-owned execution semantics fingerprint; never an executable payload.
    pub revision: u64,
    pub unavailable: Option<String>,
    pub id: CommandId,
    pub name: String,
    pub aliases: Vec<String>,
    pub summary: String,
    pub arguments: Arguments,
    pub execution: Execution,
}

impl Descriptor {
    pub fn bytes(&self) -> usize {
        128 + self.unavailable.as_ref().map_or(0, String::len)
            + self.id.source.len()
            + self.id.local.len()
            + self.name.len()
            + self.summary.len()
            + self.aliases.iter().map(String::len).sum::<usize>()
            + match &self.arguments {
                Arguments::None => 0,
                Arguments::TailText { hint, .. } => hint.len(),
            }
    }

    pub fn accepts(&self, arguments: &str) -> bool {
        match &self.arguments {
            Arguments::None => arguments.trim().is_empty(),
            Arguments::TailText { required, .. } => !required || !arguments.trim().is_empty(),
        }
    }

    pub fn matches_name(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }
}

pub fn host_commands() -> Vec<Descriptor> {
    [
        (
            "new",
            "conduit:new",
            "commands.new_chat",
            HostAction::NewChat,
        ),
        (
            "settings",
            "conduit:settings",
            "commands.settings",
            HostAction::Settings,
        ),
    ]
    .into_iter()
    .map(|(id, name, summary, action)| Descriptor {
        revision: 1,
        unavailable: None,
        id: CommandId {
            source: "conduit".into(),
            local: id.into(),
        },
        name: name.into(),
        aliases: vec![],
        summary: conduit::i18n::text(summary),
        arguments: Arguments::None,
        execution: Execution::Host(action),
    })
    .collect()
}

/// Contains no credentials. Configuration identities are opaque in-memory hashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandContext {
    pub workspace: PathBuf,
    pub session: Option<u64>,
    pub native_id: Option<String>,
    pub source: String,
    pub registration: u64,
    pub configuration: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub range: Range<usize>,
    pub name: String,
}

/// Byte ranges match the input control. Only the first token is interpreted.
pub fn parse(text: &str) -> Option<Query> {
    let start = text.len() - text.trim_start().len();
    let rest = text.get(start..)?.strip_prefix('/')?;
    let len = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let name = &rest[..len];
    if name.contains(['/', '\\']) {
        return None;
    }
    Some(Query {
        range: start..start + 1 + len,
        name: name.into(),
    })
}

pub fn query_at(text: &str, selection: Range<usize>) -> Option<Query> {
    let query = parse(text)?;
    (selection.is_empty()
        && selection.start > query.range.start
        && selection.start <= query.range.end)
        .then_some(query)
}

pub fn complete(text: &str, query: &Query, command: &Descriptor) -> (String, usize) {
    let mut result = text.to_owned();
    let replacement = format!("/{}", command.name);
    result.replace_range(query.range.clone(), &replacement);
    let mut caret = query.range.start + replacement.len();
    if caret == result.len() {
        result.push(' ');
        caret += 1;
    } else {
        caret += result[caret..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .map(char::len_utf8)
            .sum::<usize>();
    }
    (result, caret)
}

pub fn filter(mut items: Vec<Descriptor>, query: &str) -> Vec<Descriptor> {
    let query = query.to_lowercase();
    let rank = |item: &Descriptor| {
        std::iter::once(&item.name)
            .chain(&item.aliases)
            .map(|name| {
                let name = name.to_lowercase();
                if name == query {
                    0
                } else if name.starts_with(&query) {
                    1
                } else if name.contains(&query) {
                    2
                } else {
                    4
                }
            })
            .min()
            .unwrap_or(4)
    };
    items.retain(|item| rank(item) < 4);
    items.sort_by(|a, b| (rank(a), &a.name, &a.id).cmp(&(rank(b), &b.name, &b.id)));
    items
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub revision: u64,
    pub commands: Vec<Descriptor>,
    pub error: Option<String>,
    pub loading: bool,
    pub loaded: bool,
}

#[derive(Clone, Debug)]
pub struct Ticket {
    pub context: CommandContext,
    generation: u64,
}

#[derive(Default)]
pub struct Catalog {
    entries: VecDeque<(Ticket, Snapshot)>,
    sequence: u64,
}

impl Catalog {
    pub fn get(&self, context: &CommandContext) -> Option<&Snapshot> {
        self.entries
            .iter()
            .find(|(ticket, _)| &ticket.context == context)
            .map(|(_, s)| s)
    }

    /// One owner per context. Starting a newer run revokes all older publishers.
    pub fn begin(&mut self, context: CommandContext) -> Ticket {
        let previous = self
            .entries
            .iter()
            .position(|(t, _)| t.context == context)
            .and_then(|index| self.entries.remove(index))
            .map(|(_, s)| s);
        self.sequence += 1;
        let ticket = Ticket {
            context,
            generation: self.sequence,
        };
        let mut snapshot = previous.unwrap_or(Snapshot {
            revision: 0,
            commands: vec![],
            error: None,
            loading: false,
            loaded: false,
        });
        snapshot.loading = true;
        snapshot.error = None;
        self.entries.push_back((ticket.clone(), snapshot));
        while self.entries.len() > MAX_CONTEXTS {
            self.entries.pop_front();
        }
        ticket
    }

    pub fn bind_native(&mut self, ticket: &mut Ticket, native_id: String) {
        if let Some((stored, _)) = self
            .entries
            .iter_mut()
            .find(|(t, _)| t.generation == ticket.generation)
        {
            stored.context.native_id = Some(native_id);
            ticket.context = stored.context.clone();
        }
    }

    pub fn publish(&mut self, ticket: &Ticket, result: Result<Vec<Descriptor>, String>) -> bool {
        let Some((_, snapshot)) = self
            .entries
            .iter_mut()
            .find(|(t, _)| t.generation == ticket.generation && t.context == ticket.context)
        else {
            return false;
        };
        snapshot.loading = false;
        let result = result.and_then(|items| {
            let mut ids = std::collections::BTreeSet::new();
            if items.len() > MAX_COMMANDS
                || items.iter().map(Descriptor::bytes).sum::<usize>() > MAX_BYTES
            {
                return Err(conduit::i18n::text("commands.catalog_limit"));
            }
            if items.iter().any(|item| {
                item.id.source != ticket.context.source
                    || item.execution != Execution::Engine
                    || !valid_name(&item.name)
                    || !ids.insert(item.id.clone())
            }) {
                return Err(conduit::i18n::text("commands.invalid_catalog"));
            }
            Ok(items)
        });
        match result {
            Ok(commands) => {
                snapshot.commands = commands;
                snapshot.revision += 1;
                snapshot.loaded = true;
                snapshot.error = None;
            }
            Err(error) => snapshot.error = Some(error),
        }
        true
    }

    pub fn finish(&mut self, ticket: &Ticket) {
        if let Some((_, snapshot)) = self
            .entries
            .iter_mut()
            .find(|(t, _)| t.generation == ticket.generation)
        {
            snapshot.loading = false;
            if !snapshot.loaded && snapshot.error.is_none() {
                snapshot.error = Some(conduit::i18n::text("commands.no_catalog"));
            }
        }
    }

    pub fn prepare(
        &self,
        context: &CommandContext,
        selected: &Descriptor,
        text: String,
    ) -> Result<CommandCall, String> {
        if let Some(reason) = &selected.unavailable {
            return Err(reason.clone());
        }
        let snapshot = self
            .get(context)
            .ok_or_else(|| conduit::i18n::text("commands.stale"))?;
        let current = snapshot.commands.iter().find(|item| item.id == selected.id);
        if current != Some(selected) {
            return Err(conduit::i18n::text("commands.stale"));
        }
        let query = parse(&text).ok_or_else(|| conduit::i18n::text("commands.unknown"))?;
        if !selected.matches_name(&query.name) || !selected.accepts(&text[query.range.end..]) {
            return Err(conduit::i18n::text("commands.arguments"));
        }
        Ok(CommandCall {
            context: context.clone(),
            descriptor: selected.clone(),
            revision: snapshot.revision,
            text,
        })
    }
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\'))
}

/// Created only after catalog/target validation; never holds arbitrary RPC or shell input.
#[derive(Clone, Debug)]
pub struct CommandCall {
    context: CommandContext,
    descriptor: Descriptor,
    revision: u64,
    text: String,
}

impl CommandCall {
    pub fn context(&self) -> &CommandContext {
        &self.context
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn verify(&self, source: Source, native_id: Option<&str>, commands: &[Descriptor]) -> bool {
        self.context.source == source.id
            && self.context.registration == source.registration
            && self.context.native_id.as_deref() == native_id
            && self.revision > 0
            && commands.iter().any(|item| item == &self.descriptor)
    }
}

#[derive(Clone, Debug, Default)]
pub enum RequestIntent {
    #[default]
    Message,
    Command(Box<CommandCall>),
    Discover,
}

pub struct TurnInput {
    pub text: String,
    pub intent: RequestIntent,
}

impl From<String> for TurnInput {
    fn from(text: String) -> Self {
        Self {
            text,
            intent: RequestIntent::Message,
        }
    }
}

#[cfg(test)]
mod tests;
