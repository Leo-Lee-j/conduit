use super::*;
use conduit::update::{
    self as service, CheckResult, Event as UpdateEvent, LastCheck, Release, Task, install,
};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

// Signed timestamps are RFC 3339 (UTC in release-manifest). Show local wall
// time consistently, retaining compatibility with older date-only releases.
fn format_release_time(value: &str) -> String {
    if let Ok(timestamp) = chrono::DateTime::parse_from_rfc3339(value) {
        timestamp
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    } else if let Ok(date) = chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        date.format("%Y-%m-%d 00:00:00").to_string()
    } else {
        conduit::i18n::text_static("update.unknown_time").into()
    }
}

#[derive(Debug, Default, PartialEq)]
pub(super) enum Phase {
    #[default]
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Preparing,
    Failed,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum CheckOrigin {
    #[default]
    Automatic,
    Manual,
}

const AUTO_RESTART_DELAY: Duration = Duration::from_secs(30);

pub(super) struct UpdateUi {
    pub(super) phase: Phase,
    task: Option<Task>,
    release: Option<Release>,
    package: Option<PathBuf>,
    pub(super) status: String,
    next_check: Instant,
    pub(super) install_requested: bool,
    snoozed: bool,
    check_origin: CheckOrigin,
    attempt: u128,
    check_attempt: u128,
    failed_check: bool,
    repeated_check_failure: bool,
    download_percent: Option<u64>,
    restart_deadline: Option<Instant>,
    preparation: Option<mpsc::Receiver<Result<install::Pending, String>>>,
    pending: Option<install::Pending>,
    pub(super) exiting: bool,
}

impl Default for UpdateUi {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            task: None,
            release: None,
            package: None,
            status: conduit::i18n::text_static("update.you_can_check_for_updates_manually").into(),
            next_check: Instant::now() + Duration::from_secs(30),
            install_requested: false,
            snoozed: false,
            check_origin: CheckOrigin::Automatic,
            attempt: 0,
            check_attempt: 0,
            failed_check: false,
            repeated_check_failure: false,
            download_percent: None,
            restart_deadline: None,
            preparation: None,
            pending: None,
            exiting: false,
        }
    }
}

impl UpdateUi {
    pub(super) fn preparing(&self) -> bool {
        self.phase == Phase::Preparing
    }

    fn fail(&mut self, error: String) {
        self.failed_check = self.phase == Phase::Checking;
        self.phase = Phase::Failed;
        self.status = error;
        self.install_requested = false;
        self.task = None;
        self.restart_deadline = None;
    }

    fn start_attempt(&mut self) {
        self.attempt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .max(self.attempt.saturating_add(1));
        self.failed_check = false;
        self.repeated_check_failure = false;
        self.restart_deadline = None;
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Recovery {
    #[serde(default)]
    child_editors: Vec<super::subagents::navigation::SavedEditor>,
    #[serde(default)]
    child_routes: Vec<super::subagents::navigation::TaskKey>,
    #[serde(default)]
    child_pages: Vec<super::subagents::navigation::SavedPage>,
    schema: u32,
    workspace: Option<PathBuf>,
    session: Option<u64>,
    drafts: HashMap<u64, String>,
    workspace_drafts: Vec<(PathBuf, String)>,
    attachments: Vec<(PathBuf, Option<u64>, Vec<PathBuf>)>,
    commits: Vec<(PathBuf, String)>,
    #[serde(default)]
    commit_tags: Vec<(PathBuf, String)>,
}

impl ConduitApp {
    pub(super) fn show_installed_changelog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.store.updates.last_seen_version == env!("CARGO_PKG_VERSION") {
            return;
        }
        if let Ok(release) = service::installed_release(Store::path().parent().unwrap()) {
            self.updater.status = conduit::i18n::format(
                "update.updated_to_drafts_and_workspaces_restored",
                &[("arg0", release.manifest.version.to_string())],
            );
            self.updater.release = Some(release);
            self.store.updates.last_seen_version = env!("CARGO_PKG_VERSION").into();
            self.persist();
            cx.defer_in(window, |app, window, cx| app.open_about_dialog(window, cx));
        }
    }

    /// Called once at startup, before any check or download starts.
    pub(super) fn prune_update_cache(&self, cx: &mut Context<Self>) {
        if !self.writable {
            return;
        }
        let cache = Store::path().with_file_name("updates");
        let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok();
        cx.background_executor()
            .spawn(async move {
                if let Some(current) = current {
                    service::prune_cache(&cache, &current);
                }
            })
            .detach();
    }

    fn update_idle(&self) -> bool {
        !self.terminals.active()
            && self.session_control.runs.is_empty()
            && self.session_control.queue_saves.is_empty()
            && self.probes.is_empty()
            && self.opencode_model_probes.is_empty()
            && !self.plugins.packages.busy
            && self.git.operation.is_none()
            && self.git.generation.is_none()
            && self.session_control.namings.is_empty()
            && self.history_loading.is_empty()
            && self.git.probe.is_none()
            && self.git.branch_probe.is_none()
            && !self.dirty
            && !self.persistence.busy()
            && self.persistence.error.is_none()
    }

    fn check_update(&mut self, origin: CheckOrigin, cx: &mut Context<Self>) {
        if self.closing
            || matches!(
                self.updater.phase,
                Phase::Checking | Phase::Downloading | Phase::Preparing | Phase::Ready
            )
        {
            return;
        }
        self.updater.next_check = Instant::now() + Duration::from_secs(6 * 3600);
        self.updater.start_attempt();
        self.updater.check_attempt = self.updater.attempt;
        self.updater.check_origin = origin;
        self.updater.phase = Phase::Checking;
        self.updater.status = conduit::i18n::text_static("update.checking_for_updates").into();
        self.updater.release = None;
        self.updater.package = None;
        self.updater.install_requested = false;
        self.updater.snoozed = false;
        self.updater.download_percent = None;
        if service::PUBLIC_KEY.is_empty() {
            self.handle_update_event(
                UpdateEvent::Failed(conduit::i18n::text_static("update.unsupported_build").into()),
                cx,
            );
        } else {
            self.updater.task = Some(service::check());
        }
        cx.notify();
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.updater.task.is_some() {
            return;
        }
        if let Some(release) = self.updater.release.clone() {
            self.updater.start_attempt();
            self.updater.download_percent = None;
            let version = release.manifest.version.clone();
            self.updater.task = Some(service::download(
                release,
                Store::path().with_file_name("updates"),
            ));
            self.updater.phase = Phase::Downloading;
            self.updater.status =
                conduit::i18n::format("update.new_version_downloading", &[("version", version)]);
            cx.notify();
        }
    }

    pub(super) fn poll_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.updater.exiting {
            return;
        }
        if !cfg!(test)
            && !self.closing
            && self.store.updates.auto_check
            && !service::PUBLIC_KEY.is_empty()
            && Instant::now() >= self.updater.next_check
            && !matches!(
                self.updater.phase,
                Phase::Checking | Phase::Downloading | Phase::Preparing | Phase::Ready
            )
        {
            self.check_update(CheckOrigin::Automatic, cx);
        }

        let events: Vec<_> = self
            .updater
            .task
            .as_ref()
            .map(|t| t.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            if self.handle_update_event(event, cx) {
                self.download_update(cx);
            }
        }

        self.poll_update_install(window, cx);

        if let Some(result) = self
            .updater
            .preparation
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(conduit::i18n::text_static(
                    "update.update_preparation_thread_exited",
                )
                .into())),
                Err(mpsc::TryRecvError::Empty) => None,
            })
        {
            self.updater.preparation = None;
            match result {
                Ok(pending) => self.updater.pending = Some(pending),
                Err(error) => {
                    self.closing = false;
                    self.updater.fail(error);
                }
            }
            cx.notify();
        }
        if let Some(pending) = &mut self.updater.pending {
            match pending.ready() {
                Ok(true) => {
                    if !self.update_idle() {
                        if let Some(pending) = &mut self.updater.pending {
                            pending.abort();
                        }
                        self.updater.pending = None;
                        self.closing = false;
                        self.updater.phase = Phase::Ready;
                        self.updater.restart_deadline = None;
                        self.updater.status = conduit::i18n::text_static(
                            "update.new_background_tasks_detected_installation_will_start",
                        )
                        .into();
                        cx.notify();
                        return;
                    }
                    // Capture again after handoff startup, including text edited
                    // while the package was being verified on the worker.
                    if let Err(error) = self.save_update_drafts(cx) {
                        if let Some(pending) = &mut self.updater.pending {
                            pending.abort();
                        }
                        self.updater.pending = None;
                        self.closing = false;
                        self.updater.fail(conduit::i18n::format(
                            "update.save_failed_update_cancelled",
                            &[("error", format!("{:#}", error))],
                        ));
                        cx.notify();
                        return;
                    }
                    if !self.notifications.finish() {
                        return;
                    }
                    self.updater.exiting = true;
                    cx.quit();
                }
                Ok(false) => {}
                Err(error) => {
                    pending.abort();
                    self.updater.pending = None;
                    self.closing = false;
                    self.updater.fail(format!("{error:#}"));
                    cx.notify();
                }
            }
        }
    }

    fn record_update_check(&mut self, result: CheckResult) {
        self.updater.repeated_check_failure = matches!(result, CheckResult::Failed { .. })
            && self
                .store
                .updates
                .last_check
                .as_ref()
                .is_some_and(|check| matches!(check.result, CheckResult::Failed { .. }));
        self.store.updates.last_check = Some(LastCheck {
            checked_at: chrono::Utc::now().timestamp(),
            result,
        });
        if self.writable {
            self.persist();
        }
    }

    fn update_version_skipped(&self) -> bool {
        self.updater.release.as_ref().is_some_and(|release| {
            self.store.updates.skipped_version.as_deref() == Some(&release.manifest.version)
        })
    }

    /// Apply and announce a result before the caller starts any automatic download.
    fn handle_update_event(&mut self, event: UpdateEvent, cx: &mut Context<Self>) -> bool {
        let mut download = false;
        match event {
            UpdateEvent::Checked(release) => {
                self.updater.task = None;
                self.updater.release = release;
                if let Some(release) = &self.updater.release {
                    let version = release.manifest.version.clone();
                    self.record_update_check(CheckResult::Available {
                        version: version.clone(),
                    });
                    self.updater.status =
                        conduit::i18n::format("update.new_version_available", &[("arg0", version)]);
                    self.updater.phase = Phase::Available;
                    download = self.store.updates.auto_download && !self.update_version_skipped();
                } else {
                    self.record_update_check(CheckResult::Current);
                    self.updater.phase = Phase::Idle;
                    self.updater.status =
                        conduit::i18n::text_static("update.you_have_the_latest_available_version")
                            .into();
                    if self.updater.check_origin == CheckOrigin::Manual {
                        self.notify_user(
                            Notice::new(
                                NoticeSource::Update,
                                NoticeLevel::Success,
                                conduit::i18n::text_static("settings.cli.check_for_updates"),
                                self.updater.status.clone(),
                            )
                            .target(NoticeTarget::Updates)
                            .key("update/check", format!("current/{}", self.updater.attempt)),
                            cx,
                        );
                    }
                }
            }
            UpdateEvent::Progress { downloaded, total } => {
                self.updater.download_percent = (total > 0)
                    .then(|| ((downloaded as f64 / total as f64 * 100.) as u64).min(100));
                self.updater.status = conduit::i18n::format(
                    "update.downloading_mib",
                    &[
                        (
                            "arg0",
                            self.updater.download_percent.unwrap_or(0).to_string(),
                        ),
                        ("arg1", format!("{:.1}", downloaded as f64 / 1048576.)),
                        ("arg2", format!("{:.1}", total as f64 / 1048576.)),
                    ],
                );
            }
            UpdateEvent::Downloaded(path) => {
                self.updater.package = Some(path);
                self.updater.task = None;
                self.updater.phase = Phase::Ready;
                self.updater.download_percent = Some(100);
                self.updater.status = conduit::i18n::text_static(
                    "update.download_and_signature_verification_complete_ready_to",
                )
                .into();
            }
            UpdateEvent::Cancelled => {
                self.updater.task = None;
                self.updater.phase = Phase::Available;
                self.updater.download_percent = None;
                self.updater.status =
                    conduit::i18n::text_static("update.download_cancelled_you_can_download_again")
                        .into();
            }
            UpdateEvent::Failed(error) => {
                if self.updater.phase == Phase::Checking {
                    self.record_update_check(CheckResult::Failed {
                        message: error.clone(),
                    });
                }
                self.updater.fail(error);
            }
        }
        self.publish_update_notification(cx);
        cx.notify();
        download
    }

    fn poll_update_install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing || self.updater.phase != Phase::Ready {
            return;
        }
        if !self.updater.install_requested
            && (!self.store.updates.auto_install || self.updater.snoozed)
        {
            if self.updater.restart_deadline.take().is_some() {
                self.updater.status =
                    conduit::i18n::text_static("update.update_ready_you_can_install_it_later")
                        .into();
                self.refresh_notifications(cx);
            }
            return;
        }
        if !self.update_idle() {
            self.updater.restart_deadline = None;
            let waiting = if self.terminals.active() {
                conduit::i18n::text_static("update.waiting_for_terminals_to_exit_before_installing")
            } else {
                conduit::i18n::text_static(
                    "update.waiting_for_background_tasks_before_installing_you",
                )
            };
            if self.updater.status != waiting {
                self.updater.status = waiting.into();
                self.refresh_notifications(cx);
            }
            return;
        }
        if self.updater.install_requested {
            self.begin_update_install(window, cx);
            return;
        }
        if self.startup.is_some() || !window.is_window_active() {
            self.updater.restart_deadline = None;
            let waiting = conduit::i18n::text_static("update.restart_waiting_for_window");
            if self.updater.status != waiting {
                self.updater.status = waiting.into();
                self.refresh_notifications(cx);
            }
            return;
        }
        let now = Instant::now();
        let deadline = *self
            .updater
            .restart_deadline
            .get_or_insert(now + AUTO_RESTART_DELAY);
        if now >= deadline {
            self.begin_update_install(window, cx);
            return;
        }
        let seconds = deadline.duration_since(now).as_millis().div_ceil(1000);
        let status = conduit::i18n::format(
            "update.restart_countdown",
            &[("seconds", seconds.to_string())],
        );
        if self.updater.status != status {
            self.updater.status = status;
            self.refresh_notifications(cx);
        }
    }

    fn begin_update_install(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable {
            self.updater.fail(
                conduit::i18n::text_static("update.data_is_read_only_unable_to_restart").into(),
            );
            cx.notify();
            return;
        }
        let (Some(release), Some(package)) =
            (self.updater.release.clone(), self.updater.package.clone())
        else {
            return;
        };
        if self.persistence.error.is_some() {
            self.updater.fail(
                conduit::i18n::text_static("update.unable_to_save_data_update_cancelled").into(),
            );
            cx.notify();
            return;
        }
        self.closing = true;
        self.plugins.stop_instances();
        #[cfg(conduit_shell_desktop)]
        gpui_shell::ShellRoot::update(_window, cx, |root, window, cx| {
            root.clear_stopped_overlays(window, cx);
        });
        self.updater.phase = Phase::Preparing;
        self.updater.status =
            conduit::i18n::text_static("update.preparing_to_install_and_restart").into();
        let data = Store::path().parent().unwrap().to_path_buf();
        let (tx, rx) = mpsc::channel();
        self.updater.preparation = Some(rx);
        std::thread::spawn(move || {
            let result = install::prepare(&release, &package, &data).map_err(|e| format!("{e:#}"));
            if let Err(mpsc::SendError(Ok(mut pending))) = tx.send(result) {
                pending.abort();
            }
        });
        cx.notify();
    }

    fn save_update_drafts(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.save_drafts("update-recovery.json", cx)
    }

    pub(super) fn save_exit_drafts(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if !self.writable {
            return Ok(());
        }
        self.save_drafts("draft-recovery.json", cx)
    }

    fn save_drafts(&mut self, filename: &str, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        } else if let Some(target) = self.composer_target() {
            self.workspace_drafts
                .insert(target.workspace, self.input.read(cx).value().to_string());
        }
        for (id, view) in self.reference_chat_views() {
            self.drafts
                .insert(id, view.read(cx).input.read(cx).value().to_string());
        }
        anyhow::ensure!(
            !self.dirty && !self.persistence.busy() && self.persistence.error.is_none(),
            conduit::i18n::text("update.waiting_for_application_data_to_finish_saving")
        );
        let recovery = Recovery {
            child_editors: self.saved_child_editors(cx),
            child_routes: self.saved_child_routes(),
            child_pages: self.saved_child_pages(),
            schema: 1,
            workspace: self.selected_workspace_path(),
            session: self.selected_session,
            drafts: self.drafts.clone(),
            workspace_drafts: self.workspace_drafts.clone().into_iter().collect(),
            attachments: self
                .composer_files
                .iter()
                .map(|(target, files)| {
                    (
                        target.workspace.clone(),
                        target.session,
                        files.references.iter().map(|r| r.path.clone()).collect(),
                    )
                })
                .collect(),
            commit_tags: self
                .git
                .drafts
                .iter()
                .map(|(path, draft)| (path.clone(), draft.tag.read(cx).value().to_string()))
                .collect(),
            commits: self
                .git
                .drafts
                .iter()
                .map(|(path, draft)| (path.clone(), draft.input.read(cx).value().to_string()))
                .collect(),
        };
        // Store's atomic replacement protects transcripts; the recovery file is
        // synced before the old process is allowed to exit.
        let path = Store::path().with_file_name(filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        install::write_json(&path, &recovery)?;
        Ok(())
    }

    pub(super) fn restore_update_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut snapshots = ["draft-recovery.json", "update-recovery.json"];
        snapshots.sort_by_cached_key(|filename| {
            std::fs::metadata(Store::path().with_file_name(filename))
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        // A failed restart can leave both snapshots behind. The latest complete
        // snapshot wins, including removals of attachments and commit drafts.
        for filename in snapshots {
            self.restore_drafts(filename, window, cx);
        }
    }

    fn restore_drafts(&mut self, filename: &str, window: &mut Window, cx: &mut Context<Self>) {
        let path = Store::path().with_file_name(filename);
        if !path.exists() || !self.writable {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let recovery: Recovery = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(
                recovery.schema == 1,
                conduit::i18n::text("update.unknown_draft_recovery_format")
            );
            let normalize = |path: PathBuf| path.canonicalize().unwrap_or(path);
            self.drafts = recovery.drafts;
            self.workspace_drafts = recovery
                .workspace_drafts
                .into_iter()
                .map(|(path, value)| (normalize(path), value))
                .collect();
            self.composer_files.clear();
            self.git.drafts.clear();
            for (workspace, session, paths) in recovery.attachments {
                let (references, rejected) = file_ref::resolve(&paths, &workspace);
                self.composer_files.insert(
                    ComposerTarget {
                        workspace: normalize(workspace),
                        session,
                    },
                    ComposerFiles {
                        references,
                        feedback: (!rejected.is_empty()).then(|| {
                            conduit::i18n::text_static(
                                "update.some_attachments_are_inaccessible_attach_them_again",
                            )
                            .into()
                        }),
                    },
                );
            }
            let tags: HashMap<_, _> = recovery
                .commit_tags
                .into_iter()
                .map(|(path, value)| (normalize(path), value))
                .collect();
            for (path, value) in recovery.commits {
                let path = normalize(path);
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(conduit::i18n::text_static("update.enter_a_commit_message"))
                        .auto_grow(3, 3)
                });
                input.update(cx, |input, cx| input.set_value(value, window, cx));
                let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
                let tag = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(conduit::i18n::text_static("git_ui.tag_optional_e_g_v1_2_3"))
                });
                if let Some(value) = tags.get(&path) {
                    tag.update(cx, |tag, cx| tag.set_value(value.clone(), window, cx));
                }
                let tag_subscription = cx.subscribe(&tag, |_, _, _: &InputEvent, cx| cx.notify());
                self.git.drafts.insert(
                    path,
                    CommitDraft {
                        input,
                        tag,
                        _tag_subscription: tag_subscription,
                        _subscription: subscription,
                    },
                );
            }
            if let Some(workspace) = recovery.workspace.and_then(|path| {
                self.store
                    .workspaces
                    .iter()
                    .position(|w| normalize(w.path.clone()) == normalize(path.clone()))
            }) {
                let session = recovery.session.filter(|id| {
                    self.store.workspaces[workspace]
                        .sessions
                        .iter()
                        .any(|s| s.id == *id)
                });
                self.selected_workspace = None;
                self.selected_session = None;
                self.switch(workspace, session, window, cx);
            }
            self.restore_child_navigation(
                recovery.child_editors,
                recovery.child_routes,
                recovery.child_pages,
                window,
                cx,
            );
            std::fs::remove_file(&path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.notify_feedback(
                NoticeSource::Persistence,
                NoticeLevel::Error,
                conduit::i18n::format(
                    "update.draft_recovery_failed_recovery_file_preserved",
                    &[("error", format!("{:#}", error))],
                ),
                None,
                cx,
            );
        }
    }

    pub(super) fn update_content(&self, view: Entity<Self>) -> AnyElement {
        let action_view = view.clone();
        let busy = matches!(
            self.updater.phase,
            Phase::Checking | Phase::Downloading | Phase::Preparing
        );
        let mut content = v_flex()
            .gap_3()
            .child(div().text_sm().child(self.update_status()));
        content = content.child(
            div()
                .text_sm()
                .text_color(rgb(palette().muted))
                .child(self.last_update_check_text()),
        );
        if let Some(release) = &self.updater.release {
            content = content
                .child(div().font_semibold().child(conduit::i18n::format(
                    "update.version",
                    &[
                        ("arg0", release.manifest.version.to_string()),
                        (
                            "arg1",
                            format_release_time(&release.manifest.published_at).to_string(),
                        ),
                    ],
                )))
                .child(
                    div()
                        .id("update-changelog")
                        .max_h(px(240.))
                        .overflow_y_scroll()
                        .text_sm()
                        .child(TextView::markdown(
                            "update-release-notes",
                            release.manifest.notes.clone(),
                        )),
                );
        }
        let mut actions = h_flex()
            .gap_2()
            .flex_wrap()
            .justify_end()
            .child(
                Button::new("update-history")
                    .label(conduit::i18n::text_static(
                        if service::PUBLIC_KEY.is_empty() {
                            "update.open_releases"
                        } else {
                            "update.previous_releases"
                        },
                    ))
                    .small()
                    .ghost()
                    .on_click(|_, _, cx| {
                        cx.open_url("https://github.com/Leo-Lee-j/conduit/releases")
                    }),
            )
            .child(
                Button::new("about-check-update")
                    .label(conduit::i18n::text_static("settings.cli.check_for_updates"))
                    .small()
                    .secondary()
                    .disabled(
                        busy || self.closing
                            || self.updater.phase == Phase::Ready
                            || service::PUBLIC_KEY.is_empty(),
                    )
                    .on_click(move |_, _, cx| {
                        action_view.update(cx, |app, cx| app.check_update(CheckOrigin::Manual, cx))
                    }),
            );
        if self.updater.phase == Phase::Downloading {
            let view = view.clone();
            actions = actions.child(
                Button::new("cancel-update")
                    .label(conduit::i18n::text_static("update.cancel_download"))
                    .small()
                    .secondary()
                    .on_click(move |_, _, cx| {
                        view.update(cx, |app, _| {
                            if let Some(task) = &app.updater.task {
                                task.cancel();
                            }
                        })
                    }),
            );
        }
        if self.updater.release.is_some()
            && matches!(self.updater.phase, Phase::Available | Phase::Failed)
        {
            let view = view.clone();
            actions = actions.child(
                Button::new("download-update")
                    .label(conduit::i18n::text_static("update.download_update_retry"))
                    .small()
                    .disabled(self.closing)
                    .on_click(move |_, _, cx| view.update(cx, |app, cx| app.download_update(cx))),
            );
        }
        if self.updater.phase == Phase::Ready {
            if self.terminals.active() {
                let stop_view = view.clone();
                actions = actions.child(
                    Button::new("update-stop-terminals")
                        .label(conduit::i18n::text_static(
                            "update.terminate_all_terminals_and_install",
                        ))
                        .small()
                        .disabled(self.closing || !self.writable)
                        .on_click(move |_, window, cx| {
                            let stop_view = stop_view.clone();
                            window.open_dialog(cx, move |dialog, _, _| {
                                let stop_view = stop_view.clone();
                                dialog
                                    .title(conduit::i18n::text_static(
                                        "update.terminate_all_terminals_and_install_0bfc98",
                                    ))
                                    .content(|content, _, _| {
                                        content.child(conduit::i18n::text_static(
                                            "update.all_terminals_and_child_processes_will_be",
                                        ))
                                    })
                                    .on_ok(move |_, _, cx| {
                                        stop_view.update(cx, |app, cx| {
                                            app.terminals.stop_all();
                                            app.updater.install_requested = true;
                                            cx.notify();
                                        });
                                        true
                                    })
                            });
                        }),
                );
            }
            let install_view = view.clone();
            let later_view = view.clone();
            actions = actions
                .child(
                    Button::new("install-update")
                        .label(conduit::i18n::text_static("update.install_and_restart"))
                        .small()
                        .disabled(self.closing || !self.writable)
                        .on_click(move |_, _, cx| {
                            install_view.update(cx, |app, cx| {
                                app.request_update_install(cx);
                            })
                        }),
                )
                .child(
                    Button::new("later-update")
                        .label(conduit::i18n::text_static("update.later"))
                        .small()
                        .secondary()
                        .on_click(move |_, _, cx| {
                            later_view.update(cx, |app, cx| {
                                app.snooze_update(cx);
                            })
                        }),
                );
        }
        if self.updater.phase == Phase::Available {
            let view = view.clone();
            actions = actions.child(
                Button::new("skip-update")
                    .label(conduit::i18n::text_static("update.skip_this_version"))
                    .small()
                    .secondary()
                    .disabled(self.closing || !self.writable)
                    .on_click(move |_, _, cx| {
                        view.update(cx, |app, cx| {
                            app.skip_update(cx);
                        })
                    }),
            );
        }
        content.child(actions).into_any_element()
    }

    pub(super) fn publish_update_notification(&mut self, cx: &mut Context<Self>) {
        let version = self
            .updater
            .release
            .as_ref()
            .map(|release| release.manifest.version.to_string())
            .unwrap_or_default();
        let (phase, level, title) = match self.updater.phase {
            Phase::Available
                if self.update_version_skipped()
                    && self.updater.check_origin == CheckOrigin::Automatic =>
            {
                return;
            }
            Phase::Available => (
                "available",
                NoticeLevel::Info,
                conduit::i18n::text_static("update.new_version_available_010474"),
            ),
            Phase::Ready => (
                "ready",
                NoticeLevel::Success,
                conduit::i18n::text_static("update.update_ready"),
            ),
            Phase::Failed => (
                "failed",
                if self.updater.failed_check && self.updater.check_origin == CheckOrigin::Automatic
                {
                    NoticeLevel::Warning
                } else {
                    NoticeLevel::Error
                },
                conduit::i18n::text_static("update.application_update_failed"),
            ),
            _ => return,
        };
        let event = match self.updater.phase {
            Phase::Available if self.updater.check_origin == CheckOrigin::Manual => {
                format!("available/{}", self.updater.check_attempt)
            }
            Phase::Failed => format!("failed/{}", self.updater.attempt),
            _ => phase.into(),
        };
        let state = format!("{version}/{event}");
        if self.notifications.update_state.as_ref() == Some(&state) {
            return;
        }
        self.notifications.update_state = Some(state);
        let mut notice = Notice::new(
            NoticeSource::Update,
            level,
            title,
            self.updater.status.clone(),
        )
        .target(NoticeTarget::Updates)
        .key(format!("update/{version}"), event);
        if self.updater.phase == Phase::Available {
            let message = if self.store.updates.auto_download && !self.update_version_skipped() {
                conduit::i18n::Message::new(
                    "update.new_version_downloading",
                    &[("version", version)],
                )
            } else {
                conduit::i18n::Message::new("update.new_version_available", &[("arg0", version)])
            };
            notice = notice.localized_message(message);
        }
        if self.updater.phase == Phase::Failed
            && self.updater.failed_check
            && self.updater.repeated_check_failure
            && self.updater.check_origin == CheckOrigin::Automatic
        {
            notice = notice.presentation(crate::notifications::Presentation::History);
        }
        self.notify_user(notice, cx);
    }

    pub(super) fn is_ready_update_notification(
        &self,
        record: &crate::notifications::Record,
    ) -> bool {
        self.updater.phase == Phase::Ready
            && self.updater.package.is_some()
            && record.source == NoticeSource::Update
            && record.target == Some(NoticeTarget::Updates)
            && record.seen_events.iter().any(|event| event == "ready")
            && self.updater.release.as_ref().is_some_and(|release| {
                record.key.as_deref()
                    == Some(format!("update/{}", release.manifest.version).as_str())
            })
    }

    pub(super) fn request_update_install(&mut self, cx: &mut Context<Self>) {
        if self.closing || !self.writable || self.updater.phase != Phase::Ready {
            return;
        }
        self.updater.install_requested = true;
        self.updater.snoozed = false;
        self.updater.restart_deadline = None;
        self.updater.status = conduit::i18n::text_static("update.install_requested").into();
        self.refresh_notifications(cx);
    }

    pub(super) fn snooze_update(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.updater.phase != Phase::Ready {
            return;
        }
        self.updater.install_requested = false;
        self.updater.snoozed = true;
        self.updater.restart_deadline = None;
        self.updater.status =
            conduit::i18n::text_static("update.update_ready_you_can_install_it_later").into();
        let ids: Vec<_> = self
            .notifications
            .center
            .records()
            .iter()
            .filter(|record| self.is_ready_update_notification(record))
            .map(|record| record.id)
            .collect();
        for id in ids {
            self.notifications.center.mark_read(id, unix_now());
        }
        self.refresh_notifications(cx);
    }

    fn skip_update(&mut self, cx: &mut Context<Self>) {
        if self.closing || !self.writable || self.updater.phase != Phase::Available {
            return;
        }
        if let Some(release) = &self.updater.release {
            let version = release.manifest.version.clone();
            self.store.updates.skipped_version = Some(version.clone());
            // Dismiss pending reminders for this version.
            let key = format!("update/{version}");
            let ids: Vec<_> = self
                .notifications
                .center
                .records()
                .iter()
                .filter(|record| record.key.as_deref() == Some(key.as_str()))
                .map(|record| record.id)
                .collect();
            for id in ids {
                self.notifications.center.mark_read(id, unix_now());
            }
            self.refresh_notifications(cx);
        }
        self.updater.check_origin = CheckOrigin::Automatic;
        self.updater.status =
            conduit::i18n::text_static("update.version_skipped_manual_download_remains_available")
                .into();
        self.persist();
        cx.notify();
    }

    pub(super) fn update_status(&self) -> String {
        if service::PUBLIC_KEY.is_empty() && self.updater.phase == Phase::Idle {
            conduit::i18n::text_static("update.unsupported_build").into()
        } else {
            conduit::i18n::retained(&self.updater.status).to_string()
        }
    }

    pub(super) fn last_update_check_text(&self) -> String {
        let Some(check) = &self.store.updates.last_check else {
            return conduit::i18n::text_static("update.not_checked_yet").into();
        };
        let time = chrono::DateTime::from_timestamp(check.checked_at, 0)
            .map(|value| {
                value
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| conduit::i18n::text_static("update.unknown_time").into());
        let result = match &check.result {
            CheckResult::Current => {
                conduit::i18n::text_static("update.you_have_the_latest_available_version").into()
            }
            CheckResult::Available { version } => {
                conduit::i18n::format("update.new_version_available", &[("arg0", version.clone())])
            }
            CheckResult::Failed { message } => conduit::i18n::format(
                "update.check_failed_reason",
                &[("error", conduit::i18n::retained(message).to_string())],
            ),
        };
        conduit::i18n::format("update.last_check", &[("time", time), ("result", result)])
    }

    fn update_indicator_label(&self) -> String {
        let key = match self.updater.phase {
            Phase::Checking => "update.indicator_checking",
            Phase::Available
                if !self.update_version_skipped()
                    || self.updater.check_origin == CheckOrigin::Manual =>
            {
                "update.indicator_available"
            }
            Phase::Downloading => {
                return conduit::i18n::format(
                    "update.indicator_downloading",
                    &[(
                        "percent",
                        self.updater.download_percent.unwrap_or(0).to_string(),
                    )],
                );
            }
            Phase::Ready => "update.indicator_ready",
            Phase::Preparing => "update.indicator_installing",
            Phase::Failed => "update.indicator_failed",
            _ => "update.indicator_idle",
        };
        conduit::i18n::text_static(key).into()
    }

    pub(super) fn update_indicator(&self, cx: &mut Context<Self>) -> AnyElement {
        let label = concat!("v", env!("CARGO_PKG_VERSION"));
        Button::new("title-update")
            .debug_selector(|| "title-update".into())
            .accessibility_label(format!("{label} · {}", self.update_indicator_label()))
            .label(label)
            .small()
            .ghost()
            .h(px(28.))
            .flex_shrink_0()
            .tooltip(format!(
                "{}\n{}",
                self.update_status(),
                self.last_update_check_text()
            ))
            .on_click(cx.listener(|app, _, window, cx| app.open_about_dialog(window, cx)))
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{CheckOrigin, CheckResult, Instant, UpdateEvent};
    use super::{
        ComposerFiles, ComposerTarget, ConduitApp, Duration, HashMap, PathBuf, Phase, Recovery,
        Store, Workspace, file_ref, install,
    };
    use conduit::update::{Release, manifest::Manifest};
    use gpui_kit::component::WindowExt;
    use gpui_kit::{
        AppContext as _, Modifiers, MouseButton, TestAppContext, VisualTestContext, px, size,
    };

    fn release(version: &str) -> Release {
        Release {
            manifest: Manifest {
                schema: 1,
                version: version.into(),
                target: "x86_64-pc-windows-msvc".into(),
                filename: format!("Conduit-{version}-windows-x64-setup.exe"),
                url: format!(
                    "https://github.com/Leo-Lee-j/conduit/releases/download/v{version}/Conduit-{version}-windows-x64-setup.exe"
                ),
                size: 100,
                sha256: "0".repeat(64),
                published_at: "2026-11-23".into(),
                notes: "Update test".into(),
                min_updater_version: 1,
            },
            raw: vec![],
            signature: String::new(),
        }
    }

    fn click_update_action(cx: &mut VisualTestContext, selector: &str) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.run_until_parked();
        let at = cx
            .debug_bounds(selector.to_owned().leak())
            .unwrap_or_else(|| panic!("missing {selector}"))
            .center();
        cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn update_notification_actions_wait_for_work_and_reject_stale_releases(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = crate::app::queue::tests::fixture(cx, "update-notification-actions");
        let id = cx.update(|window, cx| {
            // Toast entry animation uses wall time; exercise actions at their final position.
            cx.set_reduce_motion(true);
            app.update(cx, |app, cx| {
                app.store.ui.system_notifications = false;
                app.store.updates.auto_install = false;
                app.updater.release = Some(release("1.0.0"));
                app.history_loading.insert(PathBuf::from("busy"));
                app.handle_update_event(UpdateEvent::Downloaded(root.join("package.exe")), cx);
                assert!(app.writable);
                app.poll_notifications(window, cx);
                app.notifications.center.records()[0].id
            })
        });
        cx.simulate_resize(size(px(850.), px(500.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        // A ready reminder remains actionable beyond the ordinary toast timeout.
        cx.executor().advance_clock(Duration::from_secs(6));
        click_update_action(cx, &format!("update-toast-install-{id}"));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.updater.install_requested);
                app.poll_update_install(window, cx);
                assert!(!app.closing);
                assert!(app.updater.status.contains("等待后台任务"));
                assert_eq!(app.notifications.center.records().len(), 1);
                assert_eq!(window.notifications(cx).len(), 1);
            });
        });
        click_update_action(cx, &format!("update-toast-later-{id}"));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.updater.snoozed);
                assert!(!app.updater.install_requested);
                assert!(app.notifications.center.get(id).unwrap().read_at.is_some());
                app.toggle_notifications(window, cx);
            });
        });
        // Read history still offers the current verified update after Later.
        click_update_action(cx, &format!("update-history-install-{id}"));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                assert!(app.updater.install_requested);
                assert!(!app.updater.snoozed);
                app.snooze_update(cx);
                app.writable = false;
                app.act_on_update_notification(id, true, window, cx);
                assert!(!app.updater.install_requested);
                app.writable = true;
                app.updater.release = Some(release("2.0.0"));
                app.act_on_update_notification(id, true, window, cx);
                assert!(!app.updater.install_requested);
                app.updater.release = Some(release("1.0.0"));
                app.updater.phase = Phase::Failed;
                app.act_on_update_notification(id, true, window, cx);
                assert!(!app.updater.install_requested);
                app.updater.phase = Phase::Ready;
                app.updater.package = None;
                app.act_on_update_notification(id, true, window, cx);
                assert!(!app.updater.install_requested);
                app.history_loading.clear();
            });
        });
        assert!(cx.debug_bounds("update-notice").is_none());
    }

    #[gpui_kit::test]
    fn automatic_download_announces_discovery_and_fast_completion_reuses_one_toast(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = crate::app::queue::tests::fixture(cx, "update-events");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.system_notifications = false;
                assert!(app.store.updates.auto_download);
                assert!(app.handle_update_event(UpdateEvent::Checked(Some(release("1.0.0"))), cx));
                // Discovery is recorded before the caller can start the download worker.
                let notice = &app.notifications.center.records()[0];
                let id = notice.id;
                assert!(notice.display_message().contains("正在后台下载"));
                assert_eq!(app.notifications.center.records().len(), 1);
                app.updater.phase = Phase::Downloading;
                app.publish_update_notification(cx);
                assert_eq!(app.notifications.center.records()[0].id, id);
                app.handle_update_event(UpdateEvent::Downloaded(root.join("package.exe")), cx);
                app.poll_notifications(window, cx);
                assert_eq!(window.notifications(cx).len(), 1);
                assert_eq!(app.notifications.center.records().len(), 1);
                assert_eq!(app.notifications.center.records()[0].id, id);
                assert_eq!(
                    app.notifications.center.records()[0].display_title(),
                    conduit::i18n::text_static("update.update_ready")
                );
                let revision = app.notifications.center.revision();
                app.updater.status = "等待后台任务结束".into();
                app.publish_update_notification(cx);
                assert_eq!(app.notifications.center.revision(), revision);
                // Checking again must not discard an already verified package.
                app.check_update(CheckOrigin::Manual, cx);
                assert_eq!(app.updater.phase, Phase::Ready);
                assert!(app.updater.package.is_some());
            });
        });
    }

    #[gpui_kit::test]
    fn manual_checks_always_report_latest_and_last_result_survives_reload(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "update-last-check");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.system_notifications = false;
                app.handle_update_event(UpdateEvent::Checked(None), cx);
                app.poll_notifications(window, cx);
                assert!(window.notifications(cx).is_empty());
                assert!(app.notifications.center.records().is_empty());
                app.updater.check_origin = CheckOrigin::Manual;
            });
        });
        for _ in 0..2 {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.updater.start_attempt();
                    app.handle_update_event(UpdateEvent::Checked(None), cx);
                    app.poll_notifications(window, cx);
                    assert_eq!(window.notifications(cx).len(), 1);
                    assert!(app.last_update_check_text().contains("上次检查"));
                    app.notifications.center.mark_all_read(super::unix_now());
                    app.poll_notifications(window, cx);
                });
            });
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
            cx.update(|window, cx| assert!(window.notifications(cx).is_empty()));
        }
        cx.update(|_, cx| {
            app.update(cx, |app, _| {
                assert_eq!(app.notifications.center.records().len(), 1);
                app.persistence.flush();
                let loaded = Store::load().unwrap();
                let check = loaded.updates.last_check.unwrap();
                assert!(check.checked_at > 0);
                assert!(matches!(check.result, CheckResult::Current));
                let legacy: conduit::update::Preferences = serde_json::from_str("{}").unwrap();
                assert!(legacy.last_check.is_none());
                assert!(legacy.auto_check && legacy.auto_download && !legacy.auto_install);
            });
        });
    }

    #[gpui_kit::test]
    fn repeated_automatic_failures_stay_in_history_but_manual_retry_is_visible(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "update-errors");
        for (origin, visible, error) in [
            (CheckOrigin::Automatic, true, "offline"),
            (CheckOrigin::Automatic, false, "timeout"),
            (CheckOrigin::Manual, true, "offline again"),
        ] {
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.store.ui.system_notifications = false;
                    app.updater.start_attempt();
                    app.updater.check_origin = origin;
                    app.updater.phase = Phase::Checking;
                    app.handle_update_event(UpdateEvent::Failed(error.into()), cx);
                    app.poll_notifications(window, cx);
                    assert_eq!(!window.notifications(cx).is_empty(), visible);
                    assert_eq!(app.notifications.center.records().len(), 1);
                    assert_eq!(app.notifications.center.records()[0].message, error);
                    app.notifications.center.mark_all_read(super::unix_now());
                    app.poll_notifications(window, cx);
                });
            });
            cx.executor().advance_clock(Duration::from_secs(1));
            cx.run_until_parked();
        }
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.updater.check_origin = CheckOrigin::Automatic;
                app.handle_update_event(UpdateEvent::Checked(None), cx);
                app.updater.start_attempt();
                app.updater.phase = Phase::Checking;
                app.handle_update_event(UpdateEvent::Failed("new outage".into()), cx);
                app.poll_notifications(window, cx);
                assert_eq!(window.notifications(cx).len(), 1);
            });
        });
    }

    #[gpui_kit::test]
    fn skipped_versions_suppress_automatic_reminders_but_allow_manual_checks(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "update-skip");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.store.ui.system_notifications = false;
                app.store.updates.auto_download = false;
                app.handle_update_event(UpdateEvent::Checked(Some(release("1.0.0"))), cx);
                app.skip_update(cx);
                app.poll_notifications(window, cx);
                assert!(window.notifications(cx).is_empty());
                let revision = app.notifications.center.revision();
                app.store.updates.auto_download = true;
                assert!(!app.handle_update_event(UpdateEvent::Checked(Some(release("1.0.0"))), cx));
                app.publish_update_notification(cx);
                assert_eq!(app.notifications.center.revision(), revision);
                assert_eq!(app.update_indicator_label(), "更新");
                app.updater.check_origin = CheckOrigin::Manual;
                app.updater.start_attempt();
                app.updater.check_attempt = app.updater.attempt;
                assert!(!app.handle_update_event(UpdateEvent::Checked(Some(release("1.0.0"))), cx));
                app.poll_notifications(window, cx);
                assert_eq!(window.notifications(cx).len(), 1);
                app.updater.check_origin = CheckOrigin::Automatic;
                assert!(app.handle_update_event(UpdateEvent::Checked(Some(release("1.1.0"))), cx));
                assert_eq!(app.update_indicator_label(), "发现更新");
            });
        });
    }

    #[gpui_kit::test]
    fn restart_countdown_resets_for_work_and_later_keeps_the_update_accessible(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "update-countdown");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.updater.release = Some(release("1.0.0"));
                app.updater.phase = Phase::Ready;
                app.store.updates.auto_install = true;
                app.updater.restart_deadline = Some(Instant::now() - Duration::from_secs(1));
                app.history_loading.insert(PathBuf::from("busy"));
                app.poll_update_install(window, cx);
                assert!(app.updater.restart_deadline.is_none());
                assert!(!app.closing);
                app.history_loading.clear();
                app.snooze_update(cx);
                app.poll_update_install(window, cx);
                assert!(app.updater.snoozed);
                assert!(app.updater.restart_deadline.is_none());
                assert_eq!(app.updater.phase, Phase::Ready);
                assert_eq!(app.update_indicator_label(), "更新已就绪");
                assert!(!app.closing);
                app.request_update_install(cx);
                assert!(app.updater.install_requested);
                assert!(!app.updater.snoozed);
                // Do not launch an installer in the UI regression suite.
                app.updater.install_requested = false;
                app.store.updates.auto_install = false;
            });
        });
        cx.simulate_resize(size(px(850.), px(500.)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("title-update").is_some());
        assert!(cx.debug_bounds("update-notice").is_none());
    }

    #[gpui_kit::test]
    fn automatic_restart_gives_a_full_foreground_countdown_and_blur_cancels_it(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "update-foreground");
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.updater.phase = Phase::Ready;
                app.updater.release = Some(release("1.0.0"));
                app.store.updates.auto_install = true;
                app.persistence.flush();
                assert!(app.update_idle());
                assert!(window.is_window_active());
                app.poll_update_install(window, cx);
                let deadline = app.updater.restart_deadline.unwrap();
                assert!(deadline > Instant::now() + Duration::from_secs(29));
                assert!(app.updater.status.contains("30 秒"));
                app.poll_update_install(window, cx);
                assert_eq!(app.updater.restart_deadline, Some(deadline));
                assert!(!app.closing);
            });
        });
        cx.deactivate_window();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.poll_update_install(window, cx);
                assert!(app.updater.restart_deadline.is_none());
                assert!(app.updater.status.contains("返回此窗口"));
                assert!(!app.closing);
            });
            window.activate_window();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.poll_update_install(window, cx);
                assert!(
                    app.updater.restart_deadline.unwrap()
                        > Instant::now() + Duration::from_secs(29)
                );
                app.snooze_update(cx);
                app.poll_update_install(window, cx);
                assert!(app.updater.restart_deadline.is_none());
                assert_eq!(app.updater.phase, Phase::Ready);
                assert!(!app.closing);
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(cx.debug_bounds("title-update").is_some());
        assert!(cx.debug_bounds("update-notice").is_none());
    }

    #[test]
    fn release_time_uses_local_seconds_and_handles_legacy_dates() {
        let _locale = crate::test_support::chinese();
        use chrono::TimeZone as _;
        // Use a fixed local instant so this test also works outside UTC+8.
        let local = chrono::Local
            .with_ymd_and_hms(2026, 11, 8, 9, 5, 7)
            .single()
            .unwrap();
        let utc = local.with_timezone(&chrono::Utc).to_rfc3339();
        assert_eq!(super::format_release_time(&utc), "2026-11-08 09:05:07");
        let offset = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        assert_eq!(
            super::format_release_time(&local.with_timezone(&offset).to_rfc3339()),
            "2026-11-08 09:05:07"
        );
        assert_eq!(
            super::format_release_time("2026-11-08"),
            "2026-11-08 00:00:00"
        );
        for invalid in ["", "invalid", "2026-02-30"] {
            assert_eq!(super::format_release_time(invalid), "未知时间");
        }
    }

    #[gpui_kit::test]
    fn terminal_runtime_blocks_auto_install(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = crate::app::queue::tests::fixture(cx, "terminal-update");
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.new_terminal(crate::terminal::ShellProfile::Cmd, window, cx);
                app.updater.phase = Phase::Ready;
                app.updater.install_requested = true;
                app.poll_updates(window, cx);
                assert!(app.updater.phase == Phase::Ready);
                assert!(app.updater.status.contains("等待终端"));
                assert!(!app.closing);
                app.terminals.stop_all();
            })
        });
        let start = std::time::Instant::now();
        while cx.update(|_, cx| app.read(cx).terminals.active()) {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[gpui_kit::test]
    fn update_restart_restores_chat_commit_and_attachment_drafts(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        recovery_roundtrip(cx, true);
    }

    #[gpui_kit::test]
    fn normal_exit_restores_all_drafts_from_disk_without_dispatching(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        recovery_roundtrip(cx, false);
    }

    fn recovery_roundtrip(cx: &mut TestAppContext, updating: bool) {
        cx.update(gpui_kit::init);
        let workspace = Store::path().with_file_name("update-draft-fixture");
        std::fs::create_dir_all(&workspace).unwrap();
        let attachment = workspace.join("reference.txt");
        std::fs::write(&attachment, b"reference").unwrap();
        let mut second = super::super::tests::session();
        second.id = 2;
        let mut third = second.clone();
        third.id = 3;
        let store = Store {
            version: 2,
            workspaces: vec![Workspace {
                path: workspace.clone(),
                sessions: vec![super::super::tests::session(), second, third],
            }],
            ..Default::default()
        };
        cx.add_window_view(|window, cx| {
            let original = cx.new(|cx| ConduitApp::with_store(window, cx, Ok(store), false, false));
            original.update(cx, |app, cx| {
                app.switch(0, Some(1), window, cx);
                app.drafts.insert(2, "另一会话的草稿".into());
                app.show_chat_beside(2, window, cx);
                let right = app.reference_chat_view(2).unwrap().read(cx).input.clone();
                right.update(cx, |input, cx| {
                    input.set_value("后台右侧标签草稿", window, cx)
                });
                app.show_chat_beside(3, window, cx);
                let right = app.reference_chat_view(3).unwrap().read(cx).input.clone();
                right.update(cx, |input, cx| {
                    input.set_value("当前右侧标签草稿", window, cx)
                });
                app.workspace_drafts
                    .insert(workspace.clone(), "新会话草稿".into());
                app.queued_messages.insert(
                    2,
                    crate::store::QueuedMessage {
                        text: "确认后才发送".into(),
                        references: vec![],
                        paused: None,
                    },
                );
                app.input
                    .update(cx, |input, cx| input.set_value("未发送的草稿", window, cx));
                app.composer_files.insert(
                    ComposerTarget {
                        workspace: workspace.clone(),
                        session: Some(1),
                    },
                    ComposerFiles {
                        references: file_ref::resolve(
                            std::slice::from_ref(&attachment),
                            &workspace,
                        )
                        .0,
                        feedback: None,
                    },
                );
                app.ensure_commit_draft(window, cx);
                app.git.drafts[&workspace]
                    .input
                    .update(cx, |input, cx| input.set_value("待提交说明", window, cx));
                app.git.drafts[&workspace]
                    .tag
                    .update(cx, |input, cx| input.set_value("v0.2.0", window, cx));
                assert!(app.persist());
                app.persistence.flush();
                if updating {
                    app.save_update_drafts(cx).unwrap();
                } else {
                    app.save_exit_drafts(cx).unwrap();
                }
            });
            let restored = cx.new(|cx| {
                ConduitApp::with_store(
                    window,
                    cx,
                    Store::load().map_err(|e| e.to_string()),
                    false,
                    false,
                )
            });
            restored.update(cx, |app, cx| {
                let workspace = workspace.canonicalize().unwrap();
                assert_eq!(app.input.read(cx).value().as_str(), "未发送的草稿");
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.drafts[&2], "后台右侧标签草稿");
                assert_eq!(app.drafts[&3], "当前右侧标签草稿");
                assert_eq!(app.workspace_drafts[&workspace], "新会话草稿");
                assert_eq!(app.queued_messages[&2].text, "确认后才发送");
                assert!(app.queued_messages[&2].paused.is_some());
                assert!(app.session_control.runs.is_empty());
                assert_eq!(
                    app.git.drafts[&workspace].tag.read(cx).value().as_str(),
                    "v0.2.0"
                );
                assert_eq!(
                    app.current_files().unwrap().references[0].path,
                    attachment.canonicalize().unwrap()
                );
                assert_eq!(
                    app.git.drafts[&workspace].input.read(cx).value().as_str(),
                    "待提交说明"
                );
                assert!(
                    !Store::path()
                        .with_file_name(if updating {
                            "update-recovery.json"
                        } else {
                            "draft-recovery.json"
                        })
                        .exists()
                );
            });
            gpui_kit::component::Root::new(restored, window, cx)
        });
        std::fs::remove_file(attachment).unwrap();
    }

    #[gpui_kit::test]
    fn newest_recovery_snapshot_wins_without_reviving_removed_attachments(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = super::super::queue::tests::fixture(cx, "draft-recovery-order");
        std::fs::create_dir_all(Store::path().parent().unwrap()).unwrap();
        let attachment = root.join("removed.txt");
        std::fs::write(&attachment, "reference").unwrap();
        for latest in ["draft-recovery.json", "update-recovery.json"] {
            for filename in ["draft-recovery.json", "update-recovery.json"] {
                let newest = filename == latest;
                let recovery = Recovery {
                    schema: 1,
                    workspace: Some(root.clone()),
                    session: Some(1),
                    drafts: HashMap::from([(1, if newest { "最新草稿" } else { "旧草稿" }.into())]),
                    attachments: if newest {
                        vec![]
                    } else {
                        vec![(root.clone(), Some(1), vec![attachment.clone()])]
                    },
                    commits: if newest {
                        vec![]
                    } else {
                        vec![(root.clone(), "旧提交说明".into())]
                    },
                    ..Default::default()
                };
                let path = Store::path().with_file_name(filename);
                install::write_json(&path, &recovery).unwrap();
                std::fs::File::options()
                    .write(true)
                    .open(path)
                    .unwrap()
                    .set_times(std::fs::FileTimes::new().set_modified(
                        std::time::SystemTime::UNIX_EPOCH
                            + Duration::from_secs(if newest { 200 } else { 100 }),
                    ))
                    .unwrap();
            }
            cx.update(|window, cx| {
                app.update(cx, |app, cx| {
                    app.restore_update_drafts(window, cx);
                    assert_eq!(app.input.read(cx).value().as_str(), "最新草稿");
                    assert!(
                        app.composer_files
                            .values()
                            .all(|files| files.references.is_empty())
                    );
                    assert!(
                        app.git.drafts.values().all(|draft| draft
                            .input
                            .read(cx)
                            .value()
                            .is_empty())
                    );
                    assert!(app.session_control.runs.is_empty());
                });
            });
            assert!(!Store::path().with_file_name("draft-recovery.json").exists());
            assert!(
                !Store::path()
                    .with_file_name("update-recovery.json")
                    .exists()
            );
        }
    }

    #[gpui_kit::test]
    fn normal_exit_draft_write_failure_keeps_input_and_attachments(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, root) = super::super::queue::tests::fixture(cx, "draft-save-failure");
        let attachment = root.join("keep.txt");
        std::fs::write(&attachment, "keep").unwrap();
        let path = Store::path().with_file_name("draft-recovery.json");
        std::fs::create_dir_all(&path).unwrap();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.input
                    .update(cx, |input, cx| input.set_value("不能丢失", window, cx));
                app.attach_paths(std::slice::from_ref(&attachment), window, cx);
                app.persist();
                app.persistence.flush();
                assert!(app.save_exit_drafts(cx).is_err());
                assert_eq!(app.input.read(cx).value().as_str(), "不能丢失");
                assert_eq!(app.current_files().unwrap().references.len(), 1);
                assert!(attachment.exists());
            })
        });
        std::fs::remove_dir(path).unwrap();
    }

    #[cfg(conduit_shell_desktop)]
    #[gpui_kit::test]
    fn shell_update_preparation_stops_pending_commands_before_installer_work(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        use crate::app::plugin_ui::shell::tests::{assert_no_overlays, command, fixture, runtime};
        let (app, cx) = fixture(cx);
        command(&app, "example.workspace-overview.overlays", cx);
        command(&app, "example.workspace-overview.read-delayed", cx);
        let weak = runtime(&app, cx);
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                let saved = app
                    .plugins
                    .host_command("example.workspace-overview.increment")
                    .unwrap();
                // A missing package fails preparation without ever launching an installer.
                app.updater.package =
                    Some(Store::path().with_file_name("nonexistent-shell-test.exe"));
                app.updater.release = Some(Release {
                    manifest: Manifest {
                        schema: 1,
                        version: "0.2.0".into(),
                        target: "x86_64-pc-windows-msvc".into(),
                        filename: "nonexistent-shell-test.exe".into(),
                        url: String::new(),
                        size: 1,
                        sha256: "0".repeat(64),
                        published_at: "2026-11-05".into(),
                        notes: String::new(),
                        min_updater_version: 1,
                    },
                    raw: Vec::new(),
                    signature: String::new(),
                });
                app.begin_update_install(window, cx);
                gpui_shell::ShellRoot::update(window, cx, |root, _, _| {
                    assert_eq!(root.dialog_count(), 0);
                    assert!(root.sheet().is_none());
                    assert_eq!(root.toast_count(), 0);
                })
                .unwrap();
                assert!(app.closing && app.updater.preparing());
                assert!(!app.dispatch_plugin_command(&saved, window, cx));
            })
        });
        assert_no_overlays(cx);
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(weak.upgrade().is_none());
    }

    #[gpui_kit::test]
    fn update_install_waits_for_background_work_and_about_fits_small_window(
        cx: &mut TestAppContext,
    ) {
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, cx) = cx.add_window_view(|window, cx| {
            let app =
                cx.new(|cx| ConduitApp::with_store(window, cx, Ok(Store::default()), false, false));
            *output.borrow_mut() = Some(app.clone());
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        cx.simulate_resize(size(px(850.), px(500.)));
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.updater.phase = Phase::Ready;
                app.updater.release = Some(Release { manifest: Manifest {
                    schema: 1, version: "0.2.0".into(), target: "x86_64-pc-windows-msvc".into(),
                    filename: "Conduit-0.2.0-windows-x64-setup.exe".into(),
                    url: "https://github.com/Leo-Lee-j/conduit/releases/download/v0.2.0/Conduit-0.2.0-windows-x64-setup.exe".into(),
                    size: 1234, sha256: "0".repeat(64), published_at: "2026-11-02".into(),
                    notes: "## 新功能\n\n- 后台下载与自动安装\n- 保存中文草稿与附件\n\n".repeat(30), min_updater_version: 1
                }, raw: Vec::new(), signature: String::new() });
                app.updater.install_requested = true;
                app.history_loading.insert(PathBuf::from("busy"));
                app.poll_updates(window, cx);
                assert!(!app.closing);
                assert_eq!(app.updater.status, "等待后台任务结束后安装；可选择稍后");
                app.history_loading.clear();
                app.updater.install_requested = false;
                app.store.updates.auto_install = true;
                app.updater.snoozed = true;
                app.poll_updates(window, cx);
                assert!(!app.closing, "稍后必须阻止本次自动重启");
                app.open_about_dialog(window, cx);
            })
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(window.has_active_dialog(cx));
            let _ = window.draw(cx);
            window.close_dialog(cx);
        });
    }
}
