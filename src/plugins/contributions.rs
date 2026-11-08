use super::{
    commands::CommandDescriptor,
    context::{Scope, When},
    ids::{CommandId, ItemId, Owner, ViewId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    Navigation,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Right,
}
#[derive(Clone, Debug)]
pub struct ViewDescriptor {
    pub id: ViewId,
    pub title: String,
    pub location: Location,
    pub scope: Scope,
    pub renderer: String,
    pub when: When,
}
#[derive(Clone, Debug)]
pub enum ActivityAction {
    OpenView(ViewId),
    ExecuteCommand(CommandId),
}
#[derive(Clone, Debug)]
pub struct ActivityItem {
    pub id: ItemId,
    pub title: String,
    pub icon: String,
    pub action: ActivityAction,
    pub order: i32,
    pub when: When,
}
#[derive(Clone, Debug)]
pub struct StatusValue {
    pub text: String,
    pub tooltip: Option<String>,
}
#[derive(Clone, Debug)]
pub struct StatusItem {
    pub id: ItemId,
    pub scope: Scope,
    pub alignment: Alignment,
    pub priority: i32,
    pub initial: StatusValue,
    pub icon: Option<String>,
    pub command: Option<CommandId>,
    pub when: When,
}
/// A data-only file icon provider, registered and revoked with its owner.
#[derive(Clone, Debug)]
pub struct FileIconThemeDescriptor {
    pub id: ItemId,
    pub title: String,
}
#[derive(Clone, Debug)]
pub struct ContributionSet {
    pub owner: Owner,
    pub commands: Vec<CommandDescriptor>,
    pub views: Vec<ViewDescriptor>,
    pub activity_items: Vec<ActivityItem>,
    pub status_items: Vec<StatusItem>,
    pub file_icon_themes: Vec<FileIconThemeDescriptor>,
}
impl ContributionSet {
    pub fn empty(owner: Owner) -> Self {
        Self {
            owner,
            commands: vec![],
            views: vec![],
            activity_items: vec![],
            status_items: vec![],
            file_icon_themes: vec![],
        }
    }
}
