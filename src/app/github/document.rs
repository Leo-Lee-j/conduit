use super::*;

#[derive(Default)]
pub(in crate::app) struct GithubUi {
    pub(in crate::app) active: Option<u64>,
    pub(in crate::app) panels: HashMap<u64, Entity<PullRequestPanel>>,
    pub(in crate::app) roots: HashMap<u64, PathBuf>,
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
    pub(in crate::app) fn open_github(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self.selected_workspace_path() else {
            return;
        };
        if let Some(id) = self.github_ui.id_for(&workspace) {
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
                error: None,
                sync_result: None,
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
