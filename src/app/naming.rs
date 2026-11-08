//! Background session naming and result handling.

use super::*;

impl ConduitApp {
    /// Ask the naming engine (settings → 会话) for a short title in the
    /// background; progress and failures stay visible in the conversation toolbar.
    pub(super) fn start_session_naming(&mut self, id: u64, content: &str, cx: &mut Context<Self>) {
        if self.session_control.namings.contains_key(&id) {
            return;
        }
        self.session_control.naming_errors.remove(&id);
        let (provider, model, route) =
            match self.generation_request(super::generation::GenerationKind::Naming, Some(id)) {
                Ok(selection) => selection,
                Err(error) => {
                    self.session_control.naming_errors.insert(
                        id,
                        conduit::i18n::format(
                            "naming.naming_provider_unavailable",
                            &[("error", format!("{:#}", error))],
                        ),
                    );
                    cx.notify();
                    return;
                }
            };
        let executable = match cli::resolve_for_launch(provider, &self.store.cli) {
            Ok(cli) => cli.path,
            Err(error) => {
                self.session_control.naming_errors.insert(
                    id,
                    conduit::i18n::format(
                        "naming.unable_to_start_naming_engine",
                        &[("error", format!("{:#}", error))],
                    ),
                );
                cx.notify();
                return;
            }
        };
        let cwd = self
            .store
            .workspaces
            .iter()
            .find(|w| w.sessions.iter().any(|s| s.id == id))
            .map(|w| w.path.clone())
            .unwrap_or_else(std::env::temp_dir);
        let snippet: String = content.chars().take(1200).collect();
        let naming_prompt = conduit::i18n::format(
            "naming.give_this_conversation_a_short_title_up",
            &[("snippet", snippet.to_string())],
        );
        self.session_control.namings.insert(
            id,
            SessionNaming {
                automatic: true,
                runtime: ChatRuntime::start_text_with_route_in(
                    provider,
                    executable,
                    naming_prompt,
                    model,
                    route,
                    cwd,
                ),
                started: Instant::now(),
                parts: vec![],
                error: None,
                cleanup_error: None,
            },
        );
        cx.notify();
    }

    /// Explicit "AI 重命名" from the session menu: drop any automatic naming in
    /// flight and regenerate from the latest exchange.
    pub(super) fn ai_rename_session(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        if self
            .session(id)
            .is_some_and(|session| !session.transcript.loaded)
        {
            self.transcripts.renames.insert(id);
            self.request_transcript(id, cx);
            cx.notify();
            return;
        }
        let Some(excerpt) = self.session(id).and_then(latest_naming_excerpt) else {
            self.session_control.naming_errors.insert(
                id,
                conduit::i18n::text_static("naming.no_conversation_content_available_for_renaming")
                    .into(),
            );
            cx.notify();
            return;
        };
        if let Some(naming) = self.session_control.namings.remove(&id) {
            naming.runtime.cancel();
        }
        self.start_session_naming(id, &excerpt, cx);
        if let Some(naming) = self.session_control.namings.get_mut(&id) {
            naming.automatic = false;
        }
        cx.notify();
    }

    /// Drain naming requests. Manual renames cancel and remove pending requests.
    pub(super) fn poll_session_namings(&mut self, cx: &mut Context<Self>) {
        let mut finished = vec![];
        for (id, naming) in &mut self.session_control.namings {
            if naming.started.elapsed() > Duration::from_secs(60) && naming.error.is_none() {
                naming.error =
                    Some(conduit::i18n::text_static("naming.naming_timed_out_60_seconds").into());
                naming.runtime.cancel();
                cx.notify();
            }
            loop {
                match naming.runtime.events.try_recv() {
                    Ok(Event::Text { key, text }) => {
                        update_commit_text(&mut naming.parts, key, text, true);
                    }
                    Ok(Event::Delta { key, text }) => {
                        update_commit_text(&mut naming.parts, key, text, false);
                    }
                    Ok(Event::Approval(approval)) => {
                        naming.runtime.decide(approval.id, false);
                        naming.error = Some(
                            conduit::i18n::text_static(
                                "naming.naming_model_requested_permission_cancelled",
                            )
                            .into(),
                        );
                        naming.runtime.cancel();
                    }
                    Ok(Event::Tool { .. }) | Ok(Event::ToolInput { .. }) => {
                        naming.error = Some(
                            conduit::i18n::text_static(
                                "naming.naming_model_attempted_to_call_a_tool",
                            )
                            .into(),
                        );
                        naming.runtime.cancel();
                    }
                    Ok(Event::ErrorBanner(error)) => {
                        naming.error = Some(naming_failure_text(&error));
                        cx.notify();
                    }
                    Ok(Event::CleanupBlocked(error)) => {
                        naming.cleanup_error = Some(error.clone());
                        naming.error = Some(error.clone());
                        naming.runtime.cancel();
                        self.session_control
                            .naming_errors
                            .insert(*id, error.clone());
                        self.closing = false;
                        self.message = Some(error);
                        cx.notify();
                    }
                    Ok(Event::Finished { error, cancelled }) => {
                        if naming.error.is_none() {
                            naming.error =
                                error.map(|error| naming_failure_text(&error)).or_else(|| {
                                    cancelled.then(|| {
                                        conduit::i18n::text_static("naming.naming_cancelled").into()
                                    })
                                });
                        }
                        finished.push(*id);
                        break;
                    }
                    Ok(_) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if naming.error.is_none() {
                            naming.error = Some(
                                conduit::i18n::text_static(
                                    "naming.naming_process_exited_unexpectedly",
                                )
                                .into(),
                            );
                        }
                        finished.push(*id);
                        break;
                    }
                }
            }
        }
        for id in finished {
            let Some(naming) = self.session_control.namings.remove(&id) else {
                continue;
            };
            let SessionNaming { error, parts, .. } = naming;
            if let Some(reason) = error {
                self.session_control.naming_errors.insert(id, reason);
                cx.notify();
                continue;
            }
            let title = parts
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>()
                .join("")
                .trim()
                .lines()
                .next()
                .unwrap_or("")
                .trim_matches(|ch: char| {
                    ch == '"'
                        || ch == '\u{201c}'
                        || ch == '\u{201d}'
                        || ch == '\u{300c}'
                        || ch == '\u{300d}'
                        || ch == '\''
                })
                .trim_end_matches(|ch: char| {
                    ch == '.' || ch == '\u{3002}' || ch == '!' || ch == '\u{ff01}' || ch == '?'
                })
                .chars()
                .take(30)
                .collect::<String>();
            if title.is_empty() {
                self.session_control.naming_errors.insert(
                    id,
                    conduit::i18n::text_static("naming.the_model_returned_no_valid_title").into(),
                );
                cx.notify();
                continue;
            }
            if let Some(session) = self.session_mut(id)
                && session.has_user_messages()
            {
                session.title = title;
                self.persist();
                cx.notify();
            }
        }
    }
}
