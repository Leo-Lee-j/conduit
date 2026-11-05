use super::*;
use serde_json::{Value, json};

fn documents() -> (Value, Value) {
    (
        serde_json::from_str(include_str!(
            "../../fixtures/development-overview/gpui-shell.json"
        ))
        .unwrap(),
        serde_json::from_str(include_str!(
            "../../fixtures/development-overview/conduit-plugin.json"
        ))
        .unwrap(),
    )
}
fn parse(shell: &Value, host: &Value) -> Result<Manifest> {
    Manifest::parse(&serde_json::to_vec(shell)?, &serde_json::to_vec(host)?)
}

#[test]
fn declarations_bind_views_commands_and_derive_only_explicit_activation() {
    let (shell, host) = documents();
    let manifest = parse(&shell, &host).unwrap();
    assert_eq!(manifest.entries().count(), 2);
    assert_eq!(manifest.activation_events().len(), 5);
    assert!(
        manifest
            .activation_events()
            .contains("onCommand:example.workspace-overview.overlays")
    );
    assert_eq!(
        manifest
            .binding(&CommandId::new("example.workspace-overview.increment").unwrap())
            .unwrap()
            .command,
        "increment"
    );
    let mut explicit = host;
    explicit["activationEvents"] = json!(manifest.activation_events());
    assert!(parse(&shell, &explicit).is_ok());
}

#[test]
fn incompatible_unknown_and_cross_owner_declarations_fail_before_registration() {
    let (shell, host) = documents();
    let cases: Vec<Value> = vec![
        json!({"schemaVersion":2}),
        json!({"hostApi":"^9.0"}),
        json!({"id":"duplicate.identity"}),
        json!({"permissions":["process.execute"]}),
        json!({"activationEvents":["onStartup"]}),
    ];
    for fields in cases {
        let mut bad = host.clone();
        bad.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert!(parse(&shell, &bad).is_err(), "{bad}");
    }
    for mutate in [
        |v: &mut Value| v["contributes"]["views"][0]["id"] = json!("other.owner.view"),
        |v: &mut Value| v["contributes"]["views"][0]["scope"] = json!("window"),
        |v: &mut Value| v["contributes"]["views"][0]["renderer"] = json!("conduit.core.files"),
        |v: &mut Value| v["contributes"]["views"][0]["entry"] = json!("../escape.js"),
        |v: &mut Value| v["contributes"]["views"][0]["when"] = json!("eval(code)"),
        |v: &mut Value| {
            v["contributes"]["commands"][0]["binding"]["view"] = json!("other.owner.view")
        },
        |v: &mut Value| v["contributes"]["commands"][0]["scope"] = json!("global"),
        |v: &mut Value| {
            v["contributes"]["commands"][0]["binding"]["command"] = json!("bad command")
        },
        |v: &mut Value| v["contributes"]["activityItems"][0]["icon"] = json!("script-defined"),
        |v: &mut Value| {
            v["contributes"]["statusItems"][0]["command"] = json!("other.owner.command")
        },
        |v: &mut Value| {
            v["contributes"]["commands"][1]["id"] = v["contributes"]["commands"][0]["id"].clone()
        },
    ] {
        let mut bad = host.clone();
        mutate(&mut bad);
        assert!(parse(&shell, &bad).is_err(), "{bad}");
    }
    for mutate in [
        |v: &mut Value| v["id"] = json!("conduit.reserved"),
        |v: &mut Value| v["entry"] = json!("unbound.js"),
        |v: &mut Value| v["shell-version"] = json!("0.9.0"),
        |v: &mut Value| v["capabilities"]["storage"] = json!(true),
        |v: &mut Value| v["capabilities"]["network"] = json!({}),
        |v: &mut Value| v["dependencies"] = json!({"remote":"any"}),
    ] {
        let mut bad = shell.clone();
        mutate(&mut bad);
        assert!(parse(&bad, &host).is_err(), "{bad}");
    }
}

#[test]
fn manifest_and_argument_limits_are_checked_statically() {
    let (shell, host) = documents();
    assert!(Manifest::parse(&vec![b' '; MAX_MANIFEST_BYTES + 1], b"{}").is_err());
    let mut bad = host.clone();
    bad["contributes"]["commands"][1]["arguments"] =
        json!({"kind":"object","maxBytes":65537,"maxDepth":16});
    assert!(parse(&shell, &bad).is_err());
    bad["contributes"]["commands"][1]["arguments"]["maxBytes"] = json!(65536);
    assert!(parse(&shell, &bad).is_ok());
    let mut duplicate = serde_json::to_string(&host).unwrap();
    duplicate.insert_str(1, "\"schemaVersion\":1,");
    assert!(Manifest::parse(&serde_json::to_vec(&shell).unwrap(), duplicate.as_bytes()).is_err());
}

#[test]
fn portable_paths_reject_windows_aliases_and_traversal() {
    for path in [
        "../a.js",
        "C:/a.js",
        "/a.js",
        "a\\b.js",
        "a//b",
        "a/./b",
        "CON.js",
        "a/COM1.txt",
        "a/NUL",
        "a/evil.js:stream",
        "a./b",
        "a /b",
        "a/é.js",
    ] {
        assert!(relative_path(path).is_err(), "{path}");
    }
    assert!(relative_path("views/workspace-summary.js").is_ok());
}
