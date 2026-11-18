//! Desktop-side command bridge. HTTP handlers cannot access the Store or start a CLI.

use super::*;
use crate::{
    persistence::SaveReceipt,
    remote::{self, Fault, Host, Read, Reply, Request, Write},
};
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct RemoteUi {
    server: Option<Serving>,
    pending: Vec<PendingSave>,
    error: Option<String>,
}

struct Serving {
    host: Host,
    workspace: PathBuf,
    revision: u64,
    last_publish: Instant,
}

impl RemoteUi {
    pub(super) fn stop(&mut self) {
        if let Some(server) = self.server.take() {
            server.host.stop();
        }
    }

    pub(super) fn applies_to(&self, path: &Path) -> bool {
        self.server
            .as_ref()
            .is_some_and(|server| server.workspace == path)
    }
}

impl Drop for RemoteUi {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(in crate::app) struct RemoteSend {
    host: Host,
    operation: String,
    run: String,
}

impl RemoteSend {
    pub(in crate::app) fn complete(self, id: u64) {
        self.host.complete(
            &self.operation,
            Ok(json!({"session_id": id.to_string(), "run_id": self.run, "saved": true})),
        );
    }
    pub(in crate::app) fn fail(self, withdrawn: bool) {
        let fault = if withdrawn {
            Fault::conflict("The message was stopped before it was sent.")
        } else {
            Fault::save_failed()
        };
        self.host.complete(&self.operation, Err(fault));
    }
}

struct PendingSave {
    host: Host,
    operation: String,
    receipt: SaveReceipt,
    change: SavedChange,
}

struct MetadataBefore {
    title: String,
    pinned: bool,
    archived: bool,
}

impl From<&SessionRecord> for MetadataBefore {
    fn from(session: &SessionRecord) -> Self {
        Self {
            title: session.title.clone(),
            pinned: session.pinned,
            archived: session.archived,
        }
    }
}

enum SavedChange {
    Created {
        id: u64,
        revision: u64,
    },
    Metadata {
        id: u64,
        before: MetadataBefore,
        applied: MetadataBefore,
    },
}

impl SavedChange {
    fn id(&self) -> u64 {
        match self {
            Self::Created { id, .. } | Self::Metadata { id, .. } => *id,
        }
    }
}

fn engine_id(engine: Provider) -> String {
    format!("{engine:?}").to_ascii_lowercase()
}

fn version(host: &Host, id: u64, revision: u64) -> String {
    format!("\"{}-{id}-{revision}\"", host.epoch())
}

fn metadata_version(host: &Host, session: &SessionRecord) -> String {
    use std::hash::{Hash, Hasher};
    let mut hash = std::hash::DefaultHasher::new();
    (session.title.as_str(), session.pinned, session.archived).hash(&mut hash);
    version(host, session.id, hash.finish())
}

fn shortened(text: &str, length: usize) -> (&str, bool) {
    if text.len() <= length {
        return (text, false);
    }
    (&text[..text.floor_char_boundary(length)], true)
}

impl ConduitApp {
    fn remote_writable(&self) -> bool {
        self.writable
            && !self.closing
            && self.startup.is_none()
            && !self.updater.exiting
            && !self.updater.preparing()
    }

    pub(super) fn remote_start(
        &mut self,
        workspace: PathBuf,
        origin: &str,
        cx: &mut Context<Self>,
    ) -> std::io::Result<()> {
        if !self.remote_writable()
            || !self
                .store
                .workspaces
                .iter()
                .any(|item| item.path == workspace)
        {
            return Err(std::io::Error::other(
                "Select a workspace and wait for the desktop to finish loading.",
            ));
        }
        if self
            .remote
            .server
            .as_ref()
            .is_some_and(|server| server.host.active())
        {
            return Err(std::io::Error::other(
                "Stop the current Web control session first.",
            ));
        }
        let (host, requests) = Host::start(origin)?;
        self.remote.server = Some(Serving {
            host: host.clone(),
            workspace,
            revision: self.session_control.global_revision,
            last_publish: Instant::now(),
        });
        cx.spawn(async move |this, cx| {
            while let Ok(request) = requests.recv().await {
                if this
                    .update(cx, |app, cx| app.remote_request(&host, request, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Ok(())
    }

    fn remote_request(&mut self, host: &Host, request: Request, cx: &mut Context<Self>) {
        let index = self
            .remote
            .server
            .as_ref()
            .filter(|server| server.host.epoch() == host.epoch())
            .and_then(|server| {
                self.store
                    .workspaces
                    .iter()
                    .position(|item| item.path == server.workspace)
            });
        match request {
            Request::Read {
                device,
                query,
                reply,
            } => {
                if reply.is_closed() {
                    return;
                }
                let _ = reply.send(
                    index
                        .filter(|_| host.authorized(&device))
                        .ok_or_else(Fault::unavailable)
                        .and_then(|index| self.remote_read(host, index, query, cx)),
                );
            }
            Request::Write {
                device,
                operation,
                command,
            } => {
                let Some(index) =
                    index.filter(|_| self.remote_writable() && host.can_apply(&device, &operation))
                else {
                    host.complete(&operation, Err(Fault::unavailable()));
                    return;
                };
                match self.remote_write(host, index, &operation, command, cx) {
                    Ok(Some(result)) => host.complete(&operation, Ok(result)),
                    Ok(None) => {}
                    Err(error) => host.complete(&operation, Err(error)),
                }
            }
        }
    }

    fn remote_read(
        &mut self,
        host: &Host,
        index: usize,
        query: Read,
        cx: &mut Context<Self>,
    ) -> Reply {
        let workspace = &self.store.workspaces[index];
        match query {
            Read::System => Ok(
                json!({"api_version": "v1", "host_epoch": host.epoch(), "server_time": remote::now(),
                "writable": self.remote_writable(), "engines": Provider::ALL.map(|engine| json!({"id": engine_id(engine), "name": engine.label()}))}),
            ),
            Read::Workspaces { offset } => {
                let items = if offset == 0 {
                    vec![
                        json!({"id": host.epoch(), "name": workspace.name(), "session_count": workspace.sessions.len()}),
                    ]
                } else {
                    Vec::new()
                };
                Ok(json!({"items": items, "next_cursor": null}))
            }
            Read::Sessions {
                workspace: id,
                archived,
                offset,
            } => {
                if id != host.epoch() {
                    return Err(Fault::missing());
                }
                let mut sessions: Vec<_> = workspace
                    .sessions
                    .iter()
                    .filter(|session| session.archived == archived)
                    .collect();
                sessions.sort_unstable_by_key(|session| {
                    std::cmp::Reverse((session.pinned, session.last_activity_at(), session.id))
                });
                let items: Vec<_> = sessions
                    .iter()
                    .skip(offset)
                    .take(remote::PAGE_SIZE)
                    .map(|session| self.remote_session(host, session))
                    .collect();
                let next = offset.saturating_add(items.len());
                Ok(
                    json!({"items": items, "next_cursor": (next < sessions.len()).then(|| next.to_string())}),
                )
            }
            Read::Session(id) => Ok(self.remote_session(host, self.remote_target(index, id)?)),
            Read::Metadata(id) => {
                let session = self.remote_target(index, id)?;
                let version = metadata_version(host, session);
                Ok(
                    json!({"session_id": id.to_string(), "version": version, "title": shortened(&session.title, 512).0,
                    "pinned": session.pinned, "archived": session.archived}),
                )
            }
            Read::Messages {
                session: id,
                before,
            } => {
                let session = self.remote_target(index, id)?;
                if !session.transcript.loaded {
                    if self.transcripts.errors.contains_key(&id) {
                        return Ok(
                            json!({"items": [], "next_cursor": null, "transcript_state": "failed"}),
                        );
                    }
                    self.request_transcript(id, cx);
                    return Ok(
                        json!({"items": [], "next_cursor": null, "transcript_state": "loading"}),
                    );
                }
                let mut items = Vec::new();
                let mut cursor = before
                    .unwrap_or(session.messages.len())
                    .min(session.messages.len());
                while cursor > 0 && items.len() < remote::PAGE_SIZE {
                    cursor -= 1;
                    let item = &session.messages[cursor];
                    let kind = match item.kind {
                        ItemKind::User => "user",
                        ItemKind::Assistant => "assistant",
                        ItemKind::Error => "error",
                        _ => continue,
                    };
                    let (preview, truncated) = if item.kind == ItemKind::Error {
                        (
                            "The engine reported an error. Check the desktop for details.",
                            false,
                        )
                    } else {
                        shortened(&item.text, 32 * 1024)
                    };
                    items.push(json!({"id": item.key, "kind": kind, "text_preview": preview, "content_truncated": truncated, "created_at": item.created_at}));
                }
                items.reverse();
                Ok(
                    json!({"items": items, "next_cursor": (cursor > 0).then(|| cursor.to_string()), "transcript_state": "ready"}),
                )
            }
        }
    }

    fn remote_target(&self, workspace: usize, id: u64) -> Result<&SessionRecord, Fault> {
        self.session_control
            .locate(&self.store.workspaces, id)
            .filter(|(index, _)| *index == workspace)
            .map(|(index, row)| &self.store.workspaces[index].sessions[row])
            .ok_or_else(Fault::missing)
    }

    fn remote_session(&self, host: &Host, session: &SessionRecord) -> Value {
        let id = session.id;
        let active = self.session_control.runs.get(&id);
        let state = if let Some(run) = active {
            if run.cleanup_error.is_some() {
                "cleanup_blocked"
            } else if run.runtime.is_cancelled() {
                "cancelling"
            } else if run.pending_save.is_some() {
                "pending_save"
            } else if let Some(approval) = run.approvals.front() {
                if approval.is_user_input() {
                    "waiting_input"
                } else {
                    "waiting_approval"
                }
            } else {
                "running"
            }
        } else if self.session_busy(id) {
            "busy"
        } else {
            "idle"
        };
        json!({"id": id.to_string(), "workspace_id": host.epoch(), "title": shortened(&session.title, 512).0,
            "version": version(host, id, self.session_control.revision(id)), "engine": engine_id(session.provider),
            "archived": session.archived, "pinned": session.pinned, "state": state,
            "run_id": active.map(|run| &run.usage_key), "queued": self.queued_messages.contains_key(&id) || self.session_control.queue_saves.contains_key(&id)})
    }

    fn remote_write(
        &mut self,
        host: &Host,
        index: usize,
        operation: &str,
        command: Write,
        cx: &mut Context<Self>,
    ) -> Result<Option<Value>, Fault> {
        match command {
            Write::Create { workspace, engine } => {
                if workspace != host.epoch() {
                    return Err(Fault::missing());
                }
                let provider = Provider::ALL
                    .into_iter()
                    .find(|provider| engine_id(*provider) == engine)
                    .ok_or_else(|| {
                        Fault::new(422, "unknown_engine", "Select an available engine.")
                    })?;
                let session = self.prepare_session_with_defaults(provider, false);
                let id = session.id;
                self.next_id = self.next_id.max(id + 1);
                self.store.workspaces[index].sessions.insert(0, session);
                self.sidebar
                    .invalidate_workspace(&self.store.workspaces[index].path);
                self.session_control.changed(id);
                self.remote_save(
                    host,
                    operation,
                    SavedChange::Created {
                        id,
                        revision: self.session_control.revision(id),
                    },
                    cx,
                )?;
                cx.notify();
            }
            Write::Send {
                session: id,
                version: expected,
                text,
            } => {
                let session = self.remote_target(index, id)?;
                let provider = session.provider;
                if expected != version(host, id, self.session_control.revision(id)) {
                    return Err(stale());
                }
                if session.archived {
                    return Err(Fault::conflict(
                        "Restore this archived session before sending.",
                    ));
                }
                if self
                    .remote
                    .pending
                    .iter()
                    .any(|pending| pending.change.id() == id)
                {
                    return Err(Fault::conflict("The desktop is still saving this session."));
                }
                if self.session_busy(id)
                    || self.queued_messages.contains_key(&id)
                    || self.session_control.queue_saves.contains_key(&id)
                {
                    return Err(Fault::conflict(
                        "The session is running or has a queued desktop message.",
                    ));
                }
                if let Err(error) = self.start_turn(index, Some(id), provider, text, true, cx) {
                    self.message = Some(error.into());
                    cx.notify();
                    return Err(Fault::conflict(
                        "The desktop could not start the turn. Check the engine and session status on the desktop.",
                    ));
                }
                let run = self
                    .session_control
                    .runs
                    .get_mut(&id)
                    .expect("start_turn creates a run");
                let run_id = run.usage_key.clone();
                run.pending_save
                    .as_mut()
                    .expect("new run waits for save")
                    .remote = Some(RemoteSend {
                    host: host.clone(),
                    operation: operation.into(),
                    run: run_id.clone(),
                });
                host.saving(
                    operation,
                    json!({"session_id": id.to_string(), "run_id": run_id}),
                );
                cx.notify();
            }
            Write::Cancel { session: id, run } => {
                self.remote_target(index, id)?;
                let current = self
                    .session_control
                    .runs
                    .get(&id)
                    .filter(|current| current.usage_key == run)
                    .ok_or_else(|| {
                        Fault::conflict(
                            "That turn has already ended. Refresh before stopping another turn.",
                        )
                    })?;
                if !current.runtime.is_cancelled() {
                    self.stop_session_run(id);
                }
                cx.notify();
                return Ok(Some(
                    json!({"session_id": id.to_string(), "run_id": run, "cancel_requested": true}),
                ));
            }
            Write::Metadata {
                session: id,
                version: expected,
                patch,
            } => {
                let session = self.remote_target(index, id)?;
                if expected != metadata_version(host, session) {
                    return Err(stale());
                }
                if self
                    .session_control
                    .runs
                    .get(&id)
                    .is_some_and(|run| run.pending_save.is_some())
                    || self
                        .remote
                        .pending
                        .iter()
                        .any(|pending| pending.change.id() == id)
                {
                    return Err(Fault::conflict("The desktop is still saving this session."));
                }
                let title = patch.title.as_deref().map(str::trim);
                if title.is_some_and(|title| {
                    title.is_empty() || title.contains(['\n', '\r']) || title.chars().count() > 80
                }) || (title.is_none() && patch.archived.is_none() && patch.pinned.is_none())
                {
                    return Err(Fault::new(
                        422,
                        "invalid_metadata",
                        "Enter a single-line title of up to 80 characters or change a session option.",
                    ));
                }
                let before = MetadataBefore::from(session);
                if let Some(title) = title {
                    self.set_session_title(id, title);
                }
                if let Some(session) = self.session_mut(id) {
                    if let Some(pinned) = patch.pinned {
                        session.pinned = pinned;
                    }
                    if let Some(archived) = patch.archived {
                        session.archived = archived;
                    }
                }
                let applied =
                    MetadataBefore::from(self.session(id).expect("checked metadata target"));
                self.remote_save(
                    host,
                    operation,
                    SavedChange::Metadata {
                        id,
                        before,
                        applied,
                    },
                    cx,
                )?;
                cx.notify();
            }
        }
        Ok(None)
    }

    fn remote_save(
        &mut self,
        host: &Host,
        operation: &str,
        change: SavedChange,
        cx: &mut Context<Self>,
    ) -> Result<(), Fault> {
        let receipt = match self.persist_confirmed() {
            Ok(receipt) => receipt,
            Err(_) => {
                self.rollback_remote_change(change, cx);
                return Err(Fault::save_failed());
            }
        };
        host.saving(operation, json!({"session_id": change.id().to_string()}));
        self.remote.pending.push(PendingSave {
            host: host.clone(),
            operation: operation.into(),
            receipt,
            change,
        });
        Ok(())
    }

    fn rollback_remote_change(&mut self, change: SavedChange, cx: &mut Context<Self>) {
        match change {
            SavedChange::Created { id, revision } => {
                if let Some((index, row)) = self.session_control.locate(&self.store.workspaces, id)
                    && self.session_control.revision(id) == revision
                    && self.selected_session != Some(id)
                    && !self.open_tabs.contains(&DocumentTab::Chat(id))
                    && !self.session_busy(id)
                {
                    let workspace = &mut self.store.workspaces[index];
                    workspace.sessions.remove(row);
                    self.sidebar.invalidate_workspace(&workspace.path);
                    self.session_control.changed(id);
                }
            }
            SavedChange::Metadata {
                id,
                before,
                applied,
            } => {
                if let Some(session) = self.session_mut(id) {
                    if session.title == applied.title {
                        session.title = before.title;
                    }
                    if session.pinned == applied.pinned {
                        session.pinned = before.pinned;
                    }
                    if session.archived == applied.archived {
                        session.archived = before.archived;
                    }
                }
            }
        }
        self.dirty = true;
        cx.notify();
    }

    pub(super) fn poll_remote(&mut self, cx: &mut Context<Self>) {
        let mut index = 0;
        while index < self.remote.pending.len() {
            let Some(result) = self.remote.pending[index].receipt.poll() else {
                index += 1;
                continue;
            };
            let pending = self.remote.pending.remove(index);
            let reply = match result {
                Ok(()) => Ok(json!({"session_id": pending.change.id().to_string(), "saved": true})),
                Err(_) => {
                    self.rollback_remote_change(pending.change, cx);
                    Err(Fault::save_failed())
                }
            };
            pending.host.complete(&pending.operation, reply);
        }
        if let Some(server) = &mut self.remote.server {
            let revision = self.session_control.global_revision;
            // Coalesce token updates and publish after the desktop merges runtime events.
            if revision != server.revision
                && server.last_publish.elapsed() >= Duration::from_millis(250)
            {
                server.revision = revision;
                server.last_publish = Instant::now();
                server.host.changed();
            }
        }
    }

    fn remote_settings_text(&self, label: &str) -> String {
        conduit::i18n::text(&format!("remote.{label}"))
    }

    pub(super) fn remote_settings_section(
        &self,
        view: Entity<Self>,
        disabled: bool,
    ) -> SettingGroup {
        let server = self
            .remote
            .server
            .as_ref()
            .filter(|server| server.host.active());
        let active = server.is_some();
        let status = self.remote.error.clone().unwrap_or_else(|| {
            if let Some(server) = server {
                format!(
                    "{} · {}",
                    server.host.origin(),
                    server
                        .workspace
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
            } else {
                self.remote_settings_text("off")
            }
        });
        let setup = self.remote_settings_text("setup");
        let stop = self.remote_settings_text("stop");
        let copy = self.remote_settings_text("copy_pair");
        let ready = self.selected_workspace.is_some() && !disabled && self.remote_writable();
        settings::settings_section(self.remote_settings_text("title")).items(vec![
            SettingItem::new(
                self.remote_settings_text("status"),
                settings::value_field(status),
            ),
            SettingItem::new(
                self.remote_settings_text("access"),
                SettingField::render(move |_, _, _| {
                    let view = view.clone();
                    let second = view.clone();
                    if active {
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("remote-pair")
                                    .small()
                                    .label(copy.clone())
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |app, cx| app.copy_remote_pair(cx));
                                    }),
                            )
                            .child(
                                Button::new("remote-stop")
                                    .small()
                                    .label(stop.clone())
                                    .on_click(move |_, _, cx| {
                                        second.update(cx, |app, cx| {
                                            app.remote.stop();
                                            app.remote.error = None;
                                            cx.notify();
                                        });
                                    }),
                            )
                            .into_any_element()
                    } else {
                        Button::new("remote-start")
                            .small()
                            .label(setup.clone())
                            .disabled(!ready)
                            .on_click(move |_, window, cx| {
                                view.update(cx, |app, cx| app.remote_setup_dialog(window, cx));
                            })
                            .into_any_element()
                    }
                }),
            )
            .description(settings::settings_description(
                self.remote_settings_text("scope"),
            )),
        ])
    }

    fn copy_remote_pair(&mut self, cx: &mut Context<Self>) {
        let Some(server) = &self.remote.server else {
            return;
        };
        self.remote.error = Some(match server.host.link() {
            Ok(link) => {
                cx.write_to_clipboard(ClipboardItem::new_string(link));
                self.remote_settings_text("copied")
            }
            Err(error) => error.to_string(),
        });
        cx.notify();
    }

    fn remote_setup_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace) = self
            .selected_workspace
            .and_then(|index| self.store.workspaces.get(index))
            .map(|workspace| workspace.path.clone())
        else {
            return;
        };
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://desktop.example.ts.net"));
        let view = cx.entity();
        let title = self.remote_settings_text("title");
        let setup = self.remote_settings_text("setup");
        let hint = self.remote_settings_text("hint");
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let input = input.clone();
            let workspace = workspace.clone();
            dialog.w(px(440.)).title(title.clone()).child(
                v_flex()
                    .gap_3()
                    .child(hint.clone())
                    .child(Input::new(&input))
                    .child(
                        Button::new("remote-enable")
                            .primary()
                            .label(setup.clone())
                            .on_click(move |_, window, cx| {
                                let origin = input.read(cx).value().trim().to_owned();
                                view.update(cx, |app, cx| {
                                    match app.remote_start(workspace.clone(), &origin, cx) {
                                        Ok(()) => app.copy_remote_pair(cx),
                                        Err(error) => {
                                            app.remote.error = Some(error.to_string());
                                            cx.notify();
                                        }
                                    }
                                });
                                window.close_dialog(cx);
                            }),
                    ),
            )
        });
    }
}

fn stale() -> Fault {
    Fault::new(
        412,
        "stale_version",
        "The conversation changed on the desktop. Refresh and try again.",
    )
}
#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use super::{Host, Read, Write, metadata_version};
    use crate::app::queue::tests::{fixture, running};
    use crate::remote;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn web_control_preserves_desktop_draft_and_rejects_old_run(cx: &mut TestAppContext) {
        let _locale = crate::test_support::chinese();
        let (app, cx, _) = fixture(cx, "web-control");
        let (host, _requests) = Host::start("").unwrap();
        cx.update(|window, cx| {
            app.update(cx, |app, cx| {
                app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
                app.input.update(cx, |input, cx| {
                    input.set_value("desktop draft", window, cx);
                });
                let _other = running(app, 2);
                let foreign = app.remote_read(&host, 0, Read::Session(2), cx).unwrap_err();
                assert_eq!(foreign.http_status, 404);
                let _current = running(app, 1);
                let run = app.session_control.runs[&1].usage_key.clone();
                let stale = app
                    .remote_write(
                        &host,
                        0,
                        "old-stop",
                        Write::Cancel {
                            session: 1,
                            run: "old-run".into(),
                        },
                        cx,
                    )
                    .unwrap_err();
                assert_eq!(stale.http_status, 409);
                assert!(!app.session_control.runs[&1].runtime.is_cancelled());
                app.remote_write(
                    &host,
                    0,
                    "current-stop",
                    Write::Cancel { session: 1, run },
                    cx,
                )
                .unwrap();
                assert!(app.session_control.runs[&1].runtime.is_cancelled());
                assert!(!app.session_control.runs[&2].runtime.is_cancelled());
                app.session_control.runs.remove(&1);
                let version = metadata_version(&host, app.session(1).unwrap());
                let tabs = app.open_tabs.clone();
                app.remote_write(
                    &host,
                    0,
                    "archive",
                    Write::Metadata {
                        session: 1,
                        version,
                        patch: remote::MetadataPatch {
                            title: Some("Phone edit".into()),
                            pinned: Some(true),
                            archived: Some(true),
                        },
                    },
                    cx,
                )
                .unwrap();
                app.persistence.flush();
                app.poll_remote(cx);
                let session = app.session(1).unwrap();
                assert!(session.archived && session.pinned);
                assert_eq!(session.title, "Phone edit");
                assert_eq!(app.selected_session, Some(1));
                assert_eq!(app.open_tabs, tabs);
                assert_eq!(app.input.read(cx).value().as_ref(), "desktop draft");
            });
        });
        host.stop();
    }
}
