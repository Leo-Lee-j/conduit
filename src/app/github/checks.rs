use super::*;
use crate::github::checks::CheckSource;

impl PullRequestPanel {
    pub(super) fn open_check_task(
        &mut self,
        source: &CheckSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = (|| {
            if self.loading {
                return Err(label("github.check_task_unavailable").to_owned());
            }
            let pr = self
                .snapshot
                .as_ref()
                .ok_or_else(|| label("github.check_task_unavailable").to_owned())?;
            let index = (0..pr.checks.len())
                .find(|index| pr.check_source(*index).as_ref() == Some(source))
                .ok_or_else(|| label("github.check_task_unavailable").to_owned())?;
            let host = self
                .host
                .upgrade()
                .ok_or_else(|| label("workflow.missing").to_owned())?;
            host.update(cx, |app, cx| {
                if app.closing
                    || !app
                        .store
                        .workspaces
                        .iter()
                        .any(|w| w.path == self.workspace)
                {
                    return Err(label("workflow.unavailable").to_owned());
                }
                let id = if let Some(id) = app.store.workflow.check_task_id(&self.workspace, source)
                {
                    id
                } else {
                    if !app.writable || app.updater.preparing() {
                        return Err(label("workflow.unavailable").to_owned());
                    }
                    let task = pr.check_task(index, &self.workspace, unix_now())?;
                    let id = app.store.workflow.import_check_task(task)?;
                    app.persist();
                    cx.notify();
                    id
                };
                window.close_dialog(cx);
                app.open_workflow_task(id, window, cx);
                Ok(())
            })
        })();
        self.error = result.err();
        cx.notify();
    }
}
