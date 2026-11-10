use super::*;

fn context(source: &str) -> CommandContext {
    CommandContext {
        workspace: "C:/fixture".into(),
        session: Some(7),
        native_id: Some("native".into()),
        source: source.into(),
        registration: 1,
        configuration: 1,
    }
}

fn command(source: &str, name: &str) -> Descriptor {
    Descriptor {
        revision: 1,
        unavailable: None,
        id: CommandId {
            source: source.into(),
            local: name.into(),
        },
        name: name.into(),
        aliases: vec![],
        summary: "description".into(),
        arguments: Arguments::TailText {
            required: false,
            hint: "args".into(),
        },
        execution: Execution::Engine,
    }
}

#[test]
fn command_tokens_preserve_unicode_paths_and_arguments() {
    for text in [
        "hi /help",
        "https://example.com",
        "/tmp/file",
        "/tmp\\file",
        "```\n/help",
    ] {
        assert!(parse(text).is_none(), "{text}");
    }
    let text = "  /plugin:hi   中文  \"a b\"";
    let query = query_at(text, 7..7).unwrap();
    let (completed, caret) = complete(text, &query, &command("fake", "plugin:hello"));
    assert_eq!(completed, "  /plugin:hello   中文  \"a b\"");
    assert_eq!(&completed[caret..], "中文  \"a b\"");
    assert!(query_at(text, text.len()..text.len()).is_none());
    assert!(query_at(text, 2..5).is_none());
    assert_eq!(parse("/tmp").unwrap().name, "tmp");
}

#[test]
fn catalog_rejects_stale_publishers_and_preserves_snapshots_on_failure() {
    let mut cache = Catalog::default();
    let key = context("fake");
    let first = cache.begin(key.clone());
    let item = command("fake", "review");
    assert!(cache.publish(&first, Ok(vec![item.clone()])));
    let next = cache.begin(key.clone());
    assert_eq!(
        cache.get(&key).unwrap().commands.as_slice(),
        std::slice::from_ref(&item)
    );
    assert!(!cache.publish(&first, Ok(vec![])));
    cache.publish(&next, Err("offline".into()));
    assert_eq!(cache.get(&key).unwrap().commands, [item]);
    cache.publish(&next, Ok(vec![]));
    assert!(cache.get(&key).unwrap().commands.is_empty());
}

#[test]
fn source_identity_and_semantics_are_revalidated_before_execution() {
    let mut cache = Catalog::default();
    let key = context("independent-source");
    let ticket = cache.begin(key.clone());
    let item = command(&key.source, "plugin:review");
    cache.publish(&ticket, Ok(vec![item.clone()]));
    let call = cache
        .prepare(&key, &item, "/plugin:review 中文 \"a b\"".into())
        .unwrap();
    let source = Source {
        id: "independent-source",
        registration: 1,
    };
    assert!(call.verify(source, Some("native"), std::slice::from_ref(&item)));
    assert!(!call.verify(
        Source {
            registration: 2,
            ..source
        },
        Some("native"),
        std::slice::from_ref(&item)
    ));
    assert!(!call.verify(source, Some("other"), std::slice::from_ref(&item)));
    assert!(!call.verify(source, Some("native"), &[]));
    let mut changed = item.clone();
    changed.arguments = Arguments::None;
    cache.publish(&ticket, Ok(vec![changed]));
    assert!(cache.prepare(&key, &item, "/plugin:review".into()).is_err());
    let mut other = key;
    other.workspace = "C:/other".into();
    assert!(
        cache
            .prepare(&other, &item, "/plugin:review".into())
            .is_err()
    );
}

#[test]
fn budgets_reject_whole_snapshots_and_bound_contexts() {
    let mut cache = Catalog::default();
    let key = context("fake");
    let ticket = cache.begin(key.clone());
    let mut huge = command("fake", "large");
    huge.summary = "x".repeat(MAX_BYTES);
    cache.publish(&ticket, Ok(vec![huge]));
    assert!(cache.get(&key).unwrap().error.is_some());
    assert!(!cache.get(&key).unwrap().loaded);
    for id in 0..MAX_CONTEXTS + 2 {
        let mut key = context("fake");
        key.session = Some(id as u64);
        cache.begin(key);
    }
    assert_eq!(cache.entries.len(), MAX_CONTEXTS);
    assert!(!cache.publish(&ticket, Ok(vec![])));
}

#[test]
fn search_keeps_conflicting_identities_and_ranks_names_before_aliases_substrings() {
    let exact = command("first", "help");
    let other = command("second", "help");
    let prefix = command("first", "helpful");
    let substring = command("first", "my-help");
    let found = filter(
        vec![
            substring.clone(),
            prefix.clone(),
            other.clone(),
            exact.clone(),
        ],
        "HELP",
    );
    assert_eq!(found, [exact, other, prefix, substring]);
}

#[test]
fn required_arguments_and_native_session_binding_are_explicit() {
    let mut item = command("fake", "name");
    item.arguments = Arguments::TailText {
        required: true,
        hint: "name".into(),
    };
    assert!(!item.accepts("  "));
    assert!(item.accepts(" 中文 "));
    let mut key = context("fake");
    key.native_id = None;
    let mut cache = Catalog::default();
    let mut ticket = cache.begin(key.clone());
    cache.bind_native(&mut ticket, "confirmed".into());
    assert!(cache.get(&key).is_none());
    assert!(cache.publish(&ticket, Ok(vec![item])));
    assert_eq!(ticket.context.native_id.as_deref(), Some("confirmed"));
}
