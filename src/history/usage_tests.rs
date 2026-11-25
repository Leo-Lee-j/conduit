use super::*;
use crate::domain::Usage;
use serde_json::json;

pub(super) fn codex_lines(workspace: &Path, total: u64, last: u64) -> String {
    [
        json!({"type":"session_meta","payload":{"id":"usage-history","cwd":workspace}}),
        json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":total,"cached_input_tokens":80,"cache_write_input_tokens":30,"output_tokens":20,"total_tokens":total+20},
            "last_token_usage":{"input_tokens":last,"cached_input_tokens":10,"cache_write_input_tokens":5,"output_tokens":5,"total_tokens":last+5},
            "model_context_window":200000
        }}}),
        json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
    ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n")
}

#[test]
fn native_usage_survives_import_cold_reload_and_refresh_without_ledger_duplicates() {
    let fixture = crate::transcript::tests::Fixture::new();
    let mut store = fixture.store(0);
    let imported = parse_codex(&codex_lines(&fixture.root, 1000, 100), &fixture.root).unwrap();
    assert_eq!(imported.usage.context_tokens, Some(105));
    assert_eq!(imported.usage.history.unwrap().input, Some(100));
    assert_eq!(imported.usage.history.unwrap().cache_write, Some(5));
    assert_eq!(imported.usage.cache_write_input_tokens, Some(30));
    merge_imported(&mut store.workspaces[0], &mut 1, vec![imported.clone()]);
    store.save_to(&fixture.state).unwrap();
    let mut restored = crate::store::Store::load_from(&fixture.state).unwrap();
    let session = &restored.workspaces[0].sessions[0];
    assert!(!session.transcript.loaded);
    assert_eq!(session.usage, imported.usage);
    assert_eq!(session.usage.context_ratio(), Some(105. / 200000.));
    let loaded = crate::transcript::load(session.clone()).unwrap();
    assert_eq!(loaded.usage, imported.usage);
    let newer = parse_codex(&codex_lines(&fixture.root, 2000, 50), &fixture.root).unwrap();
    let (records, warnings) = prepare_imported(
        &restored.workspaces[0],
        vec![newer.clone()],
        &crate::transcript::directory(&fixture.state),
    );
    assert!(warnings.is_empty());
    assert_eq!(records[0].usage.total_tokens, 2020);
    assert_eq!(records[0].usage.context_tokens, Some(55));
    assert!(!records[0].transcript.loaded);
    assert!(restored.usage_ledger.observations.is_empty());

    restored.workspaces[0].sessions[0] = loaded;
    let live = Usage {
        context_tokens: Some(777),
        total_tokens: 4000,
        ..Default::default()
    };
    restored.workspaces[0].sessions[0].usage = live;
    merge_imported(&mut restored.workspaces[0], &mut 2, vec![newer]);
    assert_eq!(restored.workspaces[0].sessions[0].usage, live);
}

#[test]
fn claude_history_usage_deduplicates_message_snapshots_across_cached_appends() {
    let workspace = PathBuf::from("C:/history-usage");
    let user = json!({"type":"user","sessionId":"claude-usage","cwd":workspace,"promptSource":"sdk","message":{"content":"hello"}});
    let reply = |id: &str, input, output| {
        json!({"type":"assistant","sessionId":"claude-usage","message":{
            "id":id,"model":"claude-test[1M]","content":[{"type":"text","text":"reply"}],
            "usage":{"input_tokens":input,"cache_read_input_tokens":80,"cache_creation_input_tokens":10,"output_tokens":output}
        }})
    };
    let first = extend_claude(
        [user.to_string(), reply("first", 10, 20).to_string()].into_iter(),
        ClaudeState::default(),
    );
    let cached: ClaudeState =
        serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    let mut child = reply("child", 9000, 1000);
    child["isSidechain"] = json!(true);
    let state = extend_claude(
        [
            reply("first", 10, 1).to_string(),
            child.to_string(),
            reply("second", 30, 5).to_string(),
        ]
        .into_iter(),
        cached,
    );
    let usage = state.finish().unwrap().1.usage;
    assert_eq!(usage.input_tokens, 220);
    assert_eq!(usage.output_tokens, 25);
    assert_eq!(usage.context_tokens, Some(125));
    assert_eq!(usage.context_window, 1_000_000);
    assert!(usage.context_window_estimated);
    assert_eq!(usage.history.unwrap().input, Some(120));
    assert_eq!(usage.history.unwrap().output, Some(5));
    assert_eq!(usage.turn_elapsed_ms, None);
    assert_eq!(usage.turn_output_tokens, None);
}

#[test]
fn native_usage_missing_fields_stay_unknown_and_do_not_use_totals_as_context() {
    let mut usage = Usage::default();
    usage::codex(
        &mut usage,
        &json!({"total_token_usage":{"input_tokens":200,"output_tokens":20}}),
    );
    assert_eq!(usage.context_tokens, None);
    assert_eq!(usage.history.unwrap().input, None);
    assert!(!usage.cached_input_tokens_known);
    let mut claude = usage::Claude::default();
    claude.accept(&json!({"uuid":"block-id","message":{"usage":{"input_tokens":999}}}));
    assert_eq!(claude.finish(), Usage::default());
    claude.accept(&json!({"message":{"id":"partial","usage":{"input_tokens":10}}}));
    let partial = claude.finish();
    assert_eq!(partial.context_tokens, None);
    assert_eq!(partial.history.unwrap().input, None);
    assert_eq!(partial.history.unwrap().output, None);
}

#[test]
fn opencode_native_usage_uses_each_assistant_request_once_and_keeps_limit_unknown() {
    let mut total = Usage::default();
    usage::opencode(&mut total, &json!({"role":"user","tokens":{"input":9999}}));
    assert_eq!(total, Usage::default());
    for input in [10, 30] {
        usage::opencode(
            &mut total,
            &json!({"role":"assistant","tokens":{
                "input":input,"output":5,"reasoning":3,"cache":{"read":80,"write":10}
            }}),
        );
    }
    assert_eq!(total.input_tokens, 220);
    assert_eq!(total.output_tokens, 10);
    assert_eq!(total.context_tokens, Some(128));
    assert_eq!(total.history.unwrap().input, Some(120));
    assert_eq!(total.context_window, 0);
}
