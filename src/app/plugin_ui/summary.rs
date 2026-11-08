//! Pure host metadata copied at the UI boundary. Never traverse transcript bodies.
use crate::{app::session_controller::SessionController, domain::Workspace};

pub(super) const SESSION_LIMIT: usize = 100;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionSummary {
    pub id: u64,
    pub title: String,
    pub provider: &'static str,
    pub archived: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WorkspaceSummary {
    pub name: String,
    pub path: String,
    pub total: usize,
    pub archived: usize,
    pub running: usize,
    pub sessions: Vec<SessionSummary>,
}
impl WorkspaceSummary {
    pub fn capture(workspace: &Workspace, control: &SessionController) -> Self {
        Self {
            name: super::status::bounded_text(&workspace.name(), 256),
            path: super::status::bounded_text(&crate::app::display_path(&workspace.path), 4096),
            total: workspace.sessions.len(),
            archived: workspace.sessions.iter().filter(|s| s.archived).count(),
            running: workspace
                .sessions
                .iter()
                .filter(|s| control.runs.contains_key(&s.id))
                .count(),
            sessions: workspace
                .sessions
                .iter()
                .take(SESSION_LIMIT)
                .map(|s| SessionSummary {
                    id: s.id,
                    title: super::status::bounded_text(
                        if s.title.is_empty() {
                            conduit::i18n::text_static("plugin_ui.summary.untitled_session")
                        } else {
                            &s.title
                        },
                        256,
                    ),
                    provider: s.provider.label(),
                    archived: s.archived,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SESSION_LIMIT, WorkspaceSummary};
    use crate::{app::session_controller::SessionController, domain::Workspace};

    #[test]
    fn summary_is_bounded_sendable_metadata_independent_of_transcript_content() {
        let _locale = crate::test_support::chinese();
        fn pure_data<T: Send + Sync>() {}
        pure_data::<WorkspaceSummary>();
        let control = SessionController::default();
        let mut workspace = Workspace {
            path: std::env::temp_dir().join("plugin-summary"),
            sessions: (0..110)
                .map(|id| {
                    let mut session = crate::app::tests::session();
                    session.id = id;
                    session.title = "长标题\n".repeat(300);
                    session.archived = id % 2 == 0;
                    session
                })
                .collect(),
        };
        let summary = WorkspaceSummary::capture(&workspace, &control);
        assert_eq!(summary.total, 110);
        assert_eq!(summary.archived, 55);
        assert_eq!(summary.sessions.len(), SESSION_LIMIT);
        assert!(
            summary
                .sessions
                .iter()
                .all(|s| s.title.len() <= 256 && !s.title.contains('\n'))
        );
        for session in &mut workspace.sessions {
            session.messages.clear();
            session.transcript.loaded = !session.transcript.loaded;
        }
        assert_eq!(WorkspaceSummary::capture(&workspace, &control), summary);
        workspace.sessions[0].archived = false;
        assert_ne!(WorkspaceSummary::capture(&workspace, &control), summary);
    }
}
