use super::*;

#[derive(Default)]
pub(in crate::app) struct GithubUi {
    pub(in crate::app) active: Option<u64>,
    pub(in crate::app) panels: HashMap<u64, Entity<PullRequestPanel>>,
    pub(in crate::app) roots: HashMap<u64, PathBuf>,
    pub(in crate::app) sidebar_active: bool,
    pub(in crate::app) sidebars: HashMap<PathBuf, Entity<super::sidebar::RequestSidebar>>,
    next_id: u64,
}

impl GithubUi {
    pub(in crate::app) fn id_for(&self, workspace: &Path) -> Option<u64> {
        self.roots
            .iter()
            .find_map(|(id, path)| (path == workspace).then_some(*id))
    }
}

impl ConduitApp {
    #[cfg(test)]
    pub(in crate::app) fn open_github(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.github_ui.sidebar_active = true;
        self.show_git_navigation(cx);
    }

    pub(in crate::app) fn open_github_request(
        &mut self,
        requested: Option<(crate::github::forge::Forge, u64, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        if let Some(id) = self.github_ui.id_for(&workspace) {
            if let Some(requested) = requested {
                self.github_ui.panels[&id].update(cx, |panel, cx| {
                    if panel.requested.as_ref() != Some(&requested)
                        || panel.snapshot.as_ref().is_none_or(|pr| {
                            pr.forge != requested.0
                                || pr.number != requested.1
                                || pr.url != requested.2
                        })
                    {
                        panel.requested = Some(requested);
                        panel.snapshot = None;
                        panel.tab = 0;
                        panel.reload(cx);
                    }
                });
            }
            self.select_document(files::DocumentTab::PullRequest(id), window, cx);
            return;
        }
        let host = cx.entity();
        let panel = cx.new(|cx| {
            let mut panel = PullRequestPanel {
                host: host.downgrade(),
                workspace: workspace.clone(),
                focus: cx.focus_handle(),
                document: true,
                scrolls: Default::default(),
                snapshot: None,
                requested,
                error: None,
                sync_result: None,
                sign_in: Default::default(),
                loading: false,
                shown: 20,
                tab: 0,
                expanded_threads: Default::default(),
                cancel: Arc::new(AtomicBool::new(false)),
                _task: Task::ready(()),
                _subscription: Some(cx.observe(&host, |_, _, cx| cx.notify())),
            };
            panel.reload(cx);
            panel
        });
        self.github_ui.next_id += 1;
        let id = self.github_ui.next_id;
        self.github_ui.panels.insert(id, panel);
        self.github_ui.roots.insert(id, workspace);
        self.open_tabs.push(files::DocumentTab::PullRequest(id));
        self.select_document(files::DocumentTab::PullRequest(id), window, cx);
    }
}
