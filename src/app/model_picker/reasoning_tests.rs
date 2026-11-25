use super::{ComposerSelection, ConduitApp};
use crate::{
    domain::Provider,
    models::{self, ModelCatalog},
    providers::{ModelChannel, ReasoningEffort},
};
use gpui_kit::{AppContext as _, TestAppContext};
use std::collections::BTreeMap;

#[gpui_kit::test]
fn model_picker_reasoning_scopes_native_metadata_and_resolves_service_aliases(
    cx: &mut TestAppContext,
) {
    let _locale = crate::test_support::chinese();
    cx.update(gpui_kit::init);
    cx.add_window_view(|window, cx| {
        let app = cx.new(|cx| {
            let mut app = ConduitApp::with_store(window, cx, Ok(Default::default()), false, false);
            let native = models::reasoning::Capabilities::from_codex(&serde_json::json!({
                "supported_reasoning_levels":[{"effort":"low"},{"effort":"ultra"}],
                "default_reasoning_level":"low"
            })).unwrap();
            app.catalogs.insert(Provider::Codex, ModelCatalog {
                current: Some("gpt-5".into()),
                reasoning: BTreeMap::from([("gpt-5".into(), native.clone())]),
                ..Default::default()
            });
            let mut selection = ComposerSelection::new(Provider::Codex);
            assert_eq!(app.reasoning_capabilities(&selection), native);
            selection.channel = ModelChannel::Provider("other-endpoint".into());
            selection.model = Some("gpt-5".into());
            assert!(!app.reasoning_capabilities(&selection).levels.contains(&ReasoningEffort::Ultra));
            selection.model = Some("unknown".into());
            assert_eq!(app.reasoning_capabilities(&selection).choices(), [ReasoningEffort::Default]);
            app.store.providers.push(serde_json::from_value(serde_json::json!({
                "id":"claude-fixture", "name":"fixture", "engine":"Claude", "base_url":"https://example.invalid", "api_key_env":"", "models":[],
                "claude_models":{"default_model":"claude-sonnet-4-6","roles":{"opus":{"model":"claude-opus-4-7"}}}
            })).unwrap());
            selection.engine = Provider::Claude;
            selection.channel = ModelChannel::Provider("claude-fixture".into());
            selection.model = Some("opus[1M]".into());
            assert!(app.reasoning_capabilities(&selection).levels.contains(&ReasoningEffort::Xhigh));
            selection.model = None;
            let levels = app.reasoning_capabilities(&selection).levels;
            assert!(!levels.contains(&ReasoningEffort::Xhigh));
            assert!(levels.contains(&ReasoningEffort::Max));
            app.writable = false;
            app
        });
        gpui_kit::component::Root::new(app, window, cx)
    });
}
