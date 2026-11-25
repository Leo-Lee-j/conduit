use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "conduit-gemini-native-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        fs::create_dir(path.join("policies")).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.0.join("settings.json"));
        let _ = fs::remove_file(self.0.join("policies/admin.toml"));
        let _ = fs::remove_dir(self.0.join("policies"));
        let _ = fs::remove_dir(&self.0);
    }
}
#[test]
fn native_auth_and_settings_remain_untouched_and_jsonc_is_supported() {
    let fixture = Fixture::new();
    let path = fixture.0.join("settings.json");
    let original = r#"{
        // Credentials and native policy settings must not be copied or changed.
        "security":{"auth":{"selectedType":"gemini-api-key"}},
        "policyPaths":["C:/native/policy.toml"],
    }"#;
    fs::write(&path, original).unwrap();
    let req = super::super::tests::request();
    let run = prepare_with_paths(
        &req,
        std::slice::from_ref(&path),
        &fixture.0.join("policies"),
    )
    .unwrap();
    assert_eq!(run.environment, req.route.env);
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    let policy = fs::read_to_string(&run.args[1]).unwrap();
    assert!(!policy.contains("selectedType"));
    assert!(!run.args.iter().any(|s| s == "--policy"));
    drop(run);
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}
#[test]
fn unsafe_native_startup_and_enterprise_policy_conflicts_fail_before_launch() {
    let fixture = Fixture::new();
    let path = fixture.0.join("settings.json");
    let req = super::super::tests::request();
    for settings in [
        json!({"hooks":{"SessionStart":[{"command":"danger"}]}}),
        json!({"tools":{"discoveryCommand":"danger"}}),
        json!({"mcp":{"serverCommand":"danger"}}),
        json!({"experimental":{"enableAgents":true}}),
        json!({"experimental":{"autoMemory":true}}),
        json!({"experimental":{"gemmaModelRouter":{"enabled":true}}}),
        json!({"adminPolicyPaths":["C:/enterprise.toml"]}),
        json!({"mcpServers":{"__conduit_no_mcp_servers__":{}}}),
    ] {
        fs::write(&path, settings.to_string()).unwrap();
        assert!(
            prepare_with_paths(
                &req,
                std::slice::from_ref(&path),
                &fixture.0.join("policies")
            )
            .is_err()
        );
    }
    fs::write(&path, "{}").unwrap();
    fs::write(fixture.0.join("policies/admin.toml"), "").unwrap();
    assert!(
        prepare_with_paths(
            &req,
            std::slice::from_ref(&path),
            &fixture.0.join("policies")
        )
        .is_err()
    );
    fs::write(&path, "invalid secret config").unwrap();
    assert!(
        !read_settings(&path)
            .unwrap_err()
            .to_string()
            .contains("secret")
    );
}
