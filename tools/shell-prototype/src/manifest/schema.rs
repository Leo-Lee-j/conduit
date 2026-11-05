use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Shell {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(rename = "shell-version")]
    pub shell_version: String,
    pub entry: String,
    pub capabilities: SystemCapabilities,
    #[serde(default)]
    pub dependencies: BTreeMap<String, serde_json::Value>,
}

// The first development contract accepts no system capability requests.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SystemCapabilities {
    pub storage: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Conduit {
    pub schema_version: u32,
    pub host_api: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    pub contributes: Contributions,
    pub activation_events: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Contributions {
    pub views: Vec<View>,
    #[serde(default)]
    pub commands: Vec<Command>,
    #[serde(default)]
    pub activity_items: Vec<Activity>,
    #[serde(default)]
    pub status_items: Vec<Status>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Scope {
    Global,
    Workspace,
    Session,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Location {
    Navigation,
    Right,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Alignment {
    Left,
    Right,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct View {
    pub id: String,
    pub title: String,
    pub location: Location,
    pub scope: Scope,
    pub entry: String,
    pub when: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "camelCase")]
pub(super) enum Arguments {
    #[default]
    None,
    Object {
        #[serde(rename = "maxBytes")]
        max_bytes: usize,
        #[serde(rename = "maxDepth")]
        max_depth: usize,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Command {
    pub id: String,
    pub title: String,
    pub scope: Scope,
    #[serde(default)]
    pub arguments: Arguments,
    #[serde(default)]
    pub required_permissions: Vec<String>,
    pub binding: CommandBinding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommandBinding {
    pub view: String,
    pub command: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Activity {
    pub id: String,
    pub title: String,
    pub icon: String,
    pub action: Action,
    #[serde(default)]
    pub order: i32,
    pub when: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "camelCase")]
pub(super) enum Action {
    OpenView { view: String },
    Command { command: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Status {
    pub id: String,
    pub scope: Scope,
    pub alignment: Alignment,
    #[serde(default)]
    pub priority: i32,
    pub text: String,
    pub tooltip: Option<String>,
    pub icon: Option<String>,
    pub command: Option<String>,
    pub when: Option<String>,
}
