use super::*;
use conduit::update::{self as service, Event as UpdateEvent, Release, Task, install};
use serde::{Deserialize, Serialize};
use std::sync::mpsc;

#[derive(Default, PartialEq)]
enum Phase {
    #[default]
    Idle,
    Checking,
    Available,
    Downloading,
    Ready,
    Preparing,
    Failed,
}

pub(super) struct UpdateUi {
    phase: Phase,
    task: Option<Task>,
    release: Option<Release>,
    package: Option<PathBuf>,
    status: String,
    next_check: Instant,
    install_requested: bool,
    snoozed: bool,
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
            status: "可手动检查更新".into(),
            next_check: Instant::now() + Duration::from_secs(30),
            install_requested: false,
            snoozed: false,
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
        self.phase = Phase::Failed;
        self.status = error;
        self.install_requested = false;
        self.task = None;
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Recovery {
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
            self.updater.status =
                format!("已更新至 {}，草稿和工作区已恢复", release.manifest.version);
            self.updater.release = Some(release);
            self.store.updates.last_seen_version = env!("CARGO_PKG_VERSION").into();
            self.persist();
            cx.defer_in(window, |app, window, cx| app.open_about_dialog(window, cx));
        }
    }

    fn update_idle(&self) -> bool {
        self.session_control.runs.is_empty()
            && self.probes.is_empty()
            && self.opencode_model_probes.is_empty()
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

    fn check_update(&mut self, cx: &mut Context<Self>) {
        if self.closing
            || matches!(
                self.updater.phase,
                Phase::Checking | Phase::Downloading | Phase::Preparing
            )
        {
            return;
        }
        self.updater.next_check = Instant::now() + Duration::from_secs(6 * 3600);
        self.updater.phase = Phase::Checking;
        self.updater.status = "正在检查更新…".into();
        self.updater.release = None;
        self.updater.package = None;
        self.updater.install_requested = false;
        self.updater.snoozed = false;
        self.updater.task = Some(service::check());
        cx.notify();
    }

    fn download_update(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.updater.task.is_some() {
            return;
        }
        if let Some(release) = self.updater.release.clone() {
            self.updater.task = Some(service::download(
                release,
                Store::path().with_file_name("updates"),
            ));
            self.updater.phase = Phase::Downloading;
            self.updater.status = "正在后台下载…".into();
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
            self.check_update(cx);
        }

        let events: Vec<_> = self
            .updater
            .task
            .as_ref()
            .map(|t| t.events.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                UpdateEvent::Checked(release) => {
                    self.updater.task = None;
                    self.updater.release = release;
                    if let Some(release) = &self.updater.release {
                        self.updater.status = format!("发现新版本 {}", release.manifest.version);
                        self.updater.phase = Phase::Available;
                        if self.store.updates.auto_download
                            && self.store.updates.skipped_version.as_deref()
                                != Some(&release.manifest.version)
                        {
                            self.download_update(cx);
                        }
                    } else {
                        self.updater.phase = Phase::Idle;
                        self.updater.status = "当前已是最新可用版本".into();
                    }
                }
                UpdateEvent::Progress { downloaded, total } => {
                    self.updater.status = format!(
                        "后台下载 {:.0}% · {:.1} / {:.1} MiB",
                        downloaded as f64 / total as f64 * 100.,
                        downloaded as f64 / 1048576.,
                        total as f64 / 1048576.
                    );
                }
                UpdateEvent::Downloaded(path) => {
                    self.updater.package = Some(path);
                    self.updater.task = None;
                    self.updater.phase = Phase::Ready;
                    self.updater.status = "下载及签名验证完成，可以安装并重启".into();
                }
                UpdateEvent::Cancelled => {
                    self.updater.task = None;
                    self.updater.phase = Phase::Available;
                    self.updater.status = "下载已取消，可重新下载".into();
                }
                UpdateEvent::Failed(error) => self.updater.fail(error),
            }
            cx.notify();
        }

        if !self.closing
            && self.updater.phase == Phase::Ready
            && (self.updater.install_requested
                || (self.store.updates.auto_install && !self.updater.snoozed))
        {
            if self.update_idle() {
                self.begin_update_install(window, cx);
            } else if self.updater.status != "等待后台任务结束后安装；可选择稍后" {
                self.updater.status = "等待后台任务结束后安装；可选择稍后".into();
                cx.notify();
            }
        }

        if let Some(result) = self
            .updater
            .preparation
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err("更新准备线程已退出".into())),
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
                        self.updater.status = "检测到新的后台任务，将在任务结束后安装".into();
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
                        self.updater
                            .fail(format!("保存失败，更新已取消：{error:#}"));
                        cx.notify();
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

    fn begin_update_install(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.writable {
            self.updater.fail("当前数据不可写，不能安全重启".into());
            cx.notify();
            return;
        }
        let (Some(release), Some(package)) =
            (self.updater.release.clone(), self.updater.package.clone())
        else {
            return;
        };
        if self.persistence.error.is_some() {
            self.updater.fail("无法保存数据，更新已取消".into());
            cx.notify();
            return;
        }
        self.closing = true;
        self.updater.phase = Phase::Preparing;
        self.updater.status = "正在准备安装并重启…".into();
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
        if let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        } else if let Some(target) = self.composer_target() {
            self.workspace_drafts
                .insert(target.workspace, self.input.read(cx).value().to_string());
        }
        #[cfg(test)]
        anyhow::ensure!(self.persist(), "无法保存应用数据");
        anyhow::ensure!(
            !self.dirty && !self.persistence.busy() && self.persistence.error.is_none(),
            "等待应用数据保存完成"
        );
        let recovery = Recovery {
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
        install::write_json(
            &Store::path().with_file_name("update-recovery.json"),
            &recovery,
        )?;
        Ok(())
    }

    pub(super) fn restore_update_drafts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = Store::path().with_file_name("update-recovery.json");
        if !path.exists() || !self.writable {
            return;
        }
        let result = (|| -> anyhow::Result<()> {
            let recovery: Recovery = serde_json::from_slice(&std::fs::read(&path)?)?;
            anyhow::ensure!(recovery.schema == 1, "未知的更新恢复格式");
            self.drafts = recovery.drafts;
            self.workspace_drafts = recovery.workspace_drafts.into_iter().collect();
            for (workspace, session, paths) in recovery.attachments {
                let (references, rejected) = file_ref::resolve(&paths, &workspace);
                self.composer_files.insert(
                    ComposerTarget { workspace, session },
                    ComposerFiles {
                        references,
                        feedback: (!rejected.is_empty())
                            .then(|| "部分附件已不可访问，请重新添加".into()),
                    },
                );
            }
            let tags: HashMap<_, _> = recovery.commit_tags.into_iter().collect();
            for (path, value) in recovery.commits {
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("填写 Commit Message…")
                        .auto_grow(3, 3)
                });
                input.update(cx, |input, cx| input.set_value(value, window, cx));
                let subscription = cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify());
                let tag = cx
                    .new(|cx| InputState::new(window, cx).placeholder("Tag（可选，例如 v1.2.3）"));
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
            if let Some(workspace) = recovery
                .workspace
                .and_then(|path| self.store.workspaces.iter().position(|w| w.path == path))
            {
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
            std::fs::remove_file(&path)?;
            Ok(())
        })();
        if let Err(error) = result {
            self.message = Some(format!("更新后草稿恢复失败，恢复文件已保留：{error:#}"));
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
            .child(div().text_sm().child(self.updater.status.clone()));
        if let Some(release) = &self.updater.release {
            content = content
                .child(div().font_semibold().child(format!(
                    "版本 {} · {}",
                    release.manifest.version, release.manifest.published_at
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
                    .label("历史版本")
                    .small()
                    .ghost()
                    .on_click(|_, _, cx| {
                        cx.open_url("https://github.com/Leo-Lee-j/conduit/releases")
                    }),
            )
            .child(
                Button::new("about-check-update")
                    .label("检查更新")
                    .small()
                    .secondary()
                    .disabled(busy || self.closing)
                    .on_click(move |_, _, cx| {
                        action_view.update(cx, |app, cx| app.check_update(cx))
                    }),
            );
        if self.updater.phase == Phase::Downloading {
            let view = view.clone();
            actions = actions.child(
                Button::new("cancel-update")
                    .label("取消下载")
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
                    .label("下载更新 / 重试")
                    .small()
                    .disabled(self.closing)
                    .on_click(move |_, _, cx| view.update(cx, |app, cx| app.download_update(cx))),
            );
        }
        if self.updater.phase == Phase::Ready {
            let install_view = view.clone();
            let later_view = view.clone();
            actions = actions
                .child(
                    Button::new("install-update")
                        .label("安装并重启")
                        .small()
                        .disabled(self.closing || !self.writable)
                        .on_click(move |_, _, cx| {
                            install_view.update(cx, |app, cx| {
                                app.updater.install_requested = true;
                                cx.notify();
                            })
                        }),
                )
                .child(
                    Button::new("later-update")
                        .label("稍后")
                        .small()
                        .secondary()
                        .on_click(move |_, _, cx| {
                            later_view.update(cx, |app, cx| {
                                app.updater.install_requested = false;
                                app.updater.snoozed = true;
                                app.updater.status = "更新已就绪，可以稍后安装".into();
                                cx.notify();
                            })
                        }),
                );
        }
        if self.updater.phase == Phase::Available {
            let view = view.clone();
            actions = actions.child(
                Button::new("skip-update")
                    .label("跳过此版本")
                    .small()
                    .secondary()
                    .on_click(move |_, _, cx| {
                        view.update(cx, |app, cx| {
                            app.store.updates.skipped_version = app
                                .updater
                                .release
                                .as_ref()
                                .map(|r| r.manifest.version.clone());
                            app.updater.status = "已跳过此版本，仍可手动下载".into();
                            app.persist();
                            cx.notify();
                        })
                    }),
            );
        }
        content.child(actions).into_any_element()
    }

    pub(super) fn update_notice(&self, cx: &mut Context<Self>) -> AnyElement {
        if !matches!(
            self.updater.phase,
            Phase::Available | Phase::Downloading | Phase::Ready | Phase::Preparing
        ) && !(self.updater.phase == Phase::Failed && self.updater.release.is_some())
        {
            return div().into_any_element();
        }
        h_flex()
            .px_3()
            .py_1()
            .gap_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(div().text_size(px(12.)).child(self.updater.status.clone()))
            .child(
                Button::new("show-update")
                    .label("查看更新")
                    .small()
                    .ghost()
                    .on_click(cx.listener(|app, _, window, cx| app.open_about_dialog(window, cx))),
            )
            .into_any_element()
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{
        ComposerFiles, ComposerTarget, ConduitApp, PathBuf, Phase, Store, Workspace, file_ref,
    };
    use conduit::update::{Release, manifest::Manifest};
    use gpui_kit::component::WindowExt;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};

    #[gpui_kit::test]
    fn update_restart_restores_chat_commit_and_attachment_drafts(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let workspace = Store::path().with_file_name("update-draft-fixture");
        std::fs::create_dir_all(&workspace).unwrap();
        let attachment = workspace.join("reference.txt");
        std::fs::write(&attachment, b"reference").unwrap();
        let store = Store {
            version: 2,
            workspaces: vec![Workspace {
                path: workspace.clone(),
                sessions: vec![super::super::tests::session()],
            }],
            ..Default::default()
        };
        let copy: Store = serde_json::from_value(serde_json::to_value(&store).unwrap()).unwrap();
        cx.add_window_view(|window, cx| {
            let original = cx.new(|cx| ConduitApp::with_store(window, cx, Ok(store), false, false));
            original.update(cx, |app, cx| {
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
                app.save_update_drafts(cx).unwrap();
            });
            let restored = cx.new(|cx| ConduitApp::with_store(window, cx, Ok(copy), false, false));
            restored.update(cx, |app, cx| {
                assert_eq!(app.input.read(cx).value().as_str(), "未发送的草稿");
                assert_eq!(app.selected_session, Some(1));
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
                        .with_file_name("update-recovery.json")
                        .exists()
                );
            });
            gpui_kit::component::Root::new(restored, window, cx)
        });
        std::fs::remove_file(attachment).unwrap();
    }

    #[gpui_kit::test]
    fn update_install_waits_for_background_work_and_about_fits_small_window(
        cx: &mut TestAppContext,
    ) {
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
