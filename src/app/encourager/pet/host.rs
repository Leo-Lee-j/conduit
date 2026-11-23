use super::*;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::app::encourager) struct PetOutput {
    pub(super) reply: Option<String>,
    pub(super) error: Option<String>,
    pub(super) loading: bool,
    pub(super) enabled: bool,
    pub(super) runs: Vec<(u64, Instant, bool)>,
}

impl PetOutput {
    pub(in crate::app::encourager) fn from_app(app: &ConduitApp) -> Self {
        let mut runs: Vec<_> = app
            .session_control
            .runs
            .iter()
            .map(|(id, run)| {
                (
                    *id,
                    run.turn_started,
                    !run.approvals.is_empty() || run.cleanup_error.is_some(),
                )
            })
            .collect();
        runs.sort_unstable_by_key(|(id, _, _)| *id);
        Self {
            reply: app.encourager.reply.clone(),
            error: app.encourager.error.clone(),
            loading: app.encourager.loading,
            enabled: app.store.ui.encourager.enabled && app.writable && !app.closing,
            runs,
        }
    }
}

/// Emitted only for a validated, non-cancelled terminal event of a real chat run.
pub(crate) struct PetFeedback {
    pub(crate) failed: bool,
}
impl EventEmitter<PetFeedback> for ConduitApp {}
