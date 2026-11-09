//! Workspace/session lifecycle, history import, and navigation.

use super::*;
use conduit::plugins::{builtins, ids::ViewId};

pub(super) fn single_line_title(title: &str) -> String {
    title.replace(['\r', '\n'], " ")
}

/// The most recent user/assistant exchange, flattened for AI titling. Bounded
/// by both message count and character count so the naming prompt stays small.
pub(super) fn latest_naming_excerpt(session: &SessionRecord) -> Option<String> {
    const MAX_MESSAGES: usize = 4;
    const MAX_CHARS: usize = 800;
    let mut parts: Vec<&str> = session
        .messages
        .iter()
        .rev()
        .filter(|item| matches!(item.kind, ItemKind::User | ItemKind::Assistant))
        .map(|item| item.text.trim())
        .filter(|text| !text.is_empty())
        .take(MAX_MESSAGES)
        .collect();
    if parts.is_empty() {
        return None;
    }
    parts.reverse();
    let joined = parts.join("\n\n");
    let total = joined.chars().count();
    Some(
        joined
            .chars()
            .skip(total.saturating_sub(MAX_CHARS))
            .collect(),
    )
}

pub(super) fn naming_mode_label(mode: NamingMode) -> &'static str {
    match mode {
        NamingMode::Latest => conduit::i18n::text_static("sessions.from_latest_content"),
        NamingMode::FirstMessage => conduit::i18n::text_static("sessions.from_first_message"),
    }
}

/// Fit a naming failure into the toolbar's single line: first non-empty line,
/// capped at a readable length.
pub(super) fn naming_failure_text(error: &str) -> String {
    let line = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(conduit::i18n::text_static("sessions.unknown_error"));
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

impl ConduitApp {
    pub(super) fn session(&self, id: u64) -> Option<&SessionRecord> {
        let (w, s) = self.session_control.locate(&self.store.workspaces, id)?;
        self.store.workspaces.get(w)?.sessions.get(s)
    }
    pub(super) fn session_mut(&mut self, id: u64) -> Option<&mut SessionRecord> {
        let (w, s) = self.session_control.locate(&self.store.workspaces, id)?;
        self.session_control.changed(id);
        self.sidebar.dirty_sessions.insert(id);
        self.store.workspaces.get_mut(w)?.sessions.get_mut(s)
    }

    pub(super) fn select_fallback_session(&mut self) {
        if self.active_file.is_some() || self.active_diff.is_some() {
            if self
                .selected_session
                .is_some_and(|id| self.session(id).is_none_or(|s| s.archived))
            {
                self.selected_session = None;
            }
            return;
        }
        if self
            .selected_session
            .is_some_and(|id| self.session(id).is_some_and(|session| !session.archived))
        {
            self.push_tab(self.selected_session.unwrap());
            self.session_control
                .unread_completed
                .remove(&self.selected_session.unwrap());
            return;
        }
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            if let Some(session) = workspace.sessions.iter().find(|s| !s.archived) {
                self.selected_workspace = Some(index);
                self.selected_session = Some(session.id);
                self.session_control.unread_completed.remove(&session.id);
                self.push_tab(session.id);
                return;
            }
        }
        self.selected_session = None;
        if self.store.workspaces.is_empty() {
            self.selected_workspace = None;
        } else if self
            .selected_workspace
            .is_none_or(|index| index >= self.store.workspaces.len())
        {
            self.selected_workspace = Some(0);
        }
    }

    /// Chat tabs keep their own limit; file browsing cannot evict a conversation.
    pub(super) fn push_tab(&mut self, id: u64) {
        let tab = DocumentTab::Chat(id);
        if !self.open_tabs.contains(&tab) {
            self.open_tabs.push(tab);
        }
        let limit = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        while self.open_tabs.iter().filter(|t| t.chat().is_some()).count() > limit {
            if let Some(position) = self.open_tabs.iter().position(|tab| {
                tab.chat().is_some_and(|other| {
                    other != id && !self.session_control.runs.contains_key(&other)
                })
            }) {
                self.open_tabs.remove(position);
            } else {
                break;
            }
        }
    }

    pub(super) fn remove_tab(&mut self, id: u64) {
        self.open_tabs.retain(|tab| *tab != DocumentTab::Chat(id));
    }

    pub(super) fn toggle_pin(&mut self, id: u64, cx: &mut Context<Self>) {
        if !self.writable || self.closing {
            return;
        }
        let Some(session) = self.session_mut(id) else {
            return;
        };
        session.pinned = !session.pinned;
        self.persist();
        cx.notify();
    }

    pub(super) fn toggle_archive(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(id) {
            session.archived = !session.archived;
        }
        let archived_tabs: Vec<u64> = self
            .open_tabs
            .iter()
            .filter_map(|tab| tab.chat())
            .filter(|id| self.session(*id).is_none_or(|s| s.archived))
            .collect();
        self.open_tabs
            .retain(|tab| tab.chat().is_none_or(|id| !archived_tabs.contains(&id)));
        self.select_fallback_session();
        self.persist();
        self.sync_git_workspace(cx);
        cx.notify();
    }

    pub(super) fn open_rename_dialog(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.session(id).map(|session| session.title.clone()) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(current));
        input.read(cx).focus_handle(cx).focus(window, cx);
        self.rename_target = Some(id);
        self.rename_input = Some(input);
        cx.notify();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .w(px(420.))
                .margin_top(px(120.))
                .title(conduit::i18n::text_static("sessions.rename_session"))
                .content(move |content, _, cx| {
                    let Some(input) = view.read(cx).rename_input.clone() else {
                        return content;
                    };
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex().gap_3().child(Input::new(&input).h(px(32.))).child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("rename-cancel")
                                        .label(conduit::i18n::text_static("git_branches.cancel"))
                                        .small()
                                        .ghost()
                                        .cursor_pointer()
                                        .on_click(move |_, window, cx| {
                                            cancel.update(cx, |this, cx| {
                                                this.rename_target = None;
                                                this.rename_input = None;
                                                cx.notify();
                                            });
                                            window.close_dialog(cx);
                                        }),
                                )
                                .child(
                                    Button::new("rename-confirm")
                                        .label(conduit::i18n::text_static("git_branches.save"))
                                        .small()
                                        .primary()
                                        .cursor_pointer()
                                        .on_click(move |_, window, cx| {
                                            let value = confirm
                                                .read(cx)
                                                .rename_input
                                                .as_ref()
                                                .map(|input| input.read(cx).value().to_string())
                                                .unwrap_or_default();
                                            confirm.update(cx, |this, cx| {
                                                this.apply_rename(value, cx)
                                            });
                                            window.close_dialog(cx);
                                        }),
                                ),
                        ),
                    )
                })
        });
    }

    pub(super) fn apply_rename(&mut self, title: String, cx: &mut Context<Self>) {
        let Some(id) = self.rename_target.take() else {
            return;
        };
        self.rename_input = None;
        let title = title.trim().to_string();
        if title.is_empty() {
            self.message =
                Some(conduit::i18n::text_static("sessions.session_name_cannot_be_empty").into());
            cx.notify();
            return;
        }
        // A manual rename wins over any title the naming engine still owes us.
        if let Some(naming) = self.session_control.namings.remove(&id) {
            naming.runtime.cancel();
        }
        self.session_control.naming_errors.remove(&id);
        if let Some(session) = self.session_mut(id) {
            session.title = title.chars().take(80).collect();
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn request_delete_session(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(session) = self.session(id) else {
            return;
        };
        // Empty sessions have no history to protect; keep drafts protected.
        if session.message_count() == 0 && !self.session_has_unsent_content(id, cx) {
            self.delete_session(id, cx);
        } else {
            self.delete_target = Some(id);
            cx.notify();
        }
    }

    /// Mounted beside the requested row so dismissal never opens a modal layer.
    pub(super) fn delete_session_popover(&self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let title = self
            .session(id)
            .map(|session| {
                if session.title.is_empty() {
                    conduit::i18n::text_static("plugin_ui.summary.untitled_session").to_string()
                } else {
                    single_line_title(&session.title)
                }
            })
            .unwrap_or_default();
        let app = cx.entity();
        Popover::new(("delete-session-popover", id))
            .anchor(Anchor::TopLeft)
            .offset(px(6.))
            .open(true)
            .on_open_change(cx.listener(move |this, open, _, cx| {
                if !open && this.delete_target == Some(id) {
                    this.delete_target = None;
                    cx.notify();
                }
            }))
            .trigger(
                Button::new(("delete-session-anchor", id))
                    .ghost()
                    .w(px(0.))
                    .h(px(26.))
                    .p_0(),
            )
            .content(move |_, _, cx| {
                let cancel = app.clone();
                let confirm = app.clone();
                let cancel_popover = cx.entity().downgrade();
                let confirm_popover = cancel_popover.clone();
                v_flex()
                    .debug_selector(|| "delete-session-confirmation".into())
                    .w(px(260.))
                    .gap_2()
                    .child(
                        div()
                            .text_size(ui_text_size(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(conduit::i18n::text_static("sessions.delete_session")),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::format(
                                "sessions.permanently_delete_and_its_messages_this_cannot",
                                &[("title", title.to_string())],
                            )),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_2()
                            .child(
                                Button::new("delete-cancel")
                                    .debug_selector(|| "delete-cancel".into())
                                    .label(conduit::i18n::text_static("git_branches.cancel"))
                                    .small()
                                    .ghost()
                                    .cursor_pointer()
                                    .on_click(move |_, window, cx| {
                                        if let Some(popover) = cancel_popover.upgrade() {
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        }
                                        cancel.update(cx, |this, cx| {
                                            this.delete_target = None;
                                            cx.notify();
                                        })
                                    }),
                            )
                            .child(
                                Button::new("delete-confirm")
                                    .debug_selector(|| "delete-confirm".into())
                                    .label(conduit::i18n::text_static("git_state.deleted"))
                                    .small()
                                    .danger()
                                    .cursor_pointer()
                                    .on_click(move |_, window, cx| {
                                        let requested = confirm.read(cx).delete_target == Some(id);
                                        if let Some(popover) = confirm_popover.upgrade() {
                                            popover
                                                .update(cx, |state, cx| state.dismiss(window, cx));
                                        }
                                        if requested {
                                            confirm
                                                .update(cx, |this, cx| this.delete_session(id, cx));
                                        }
                                    }),
                            ),
                    )
            })
            .into_any_element()
    }

    /// Composer content that would be lost with the session: unsent files, a
    /// saved draft after switching away, or text still in the active composer.
    pub(super) fn session_has_unsent_content(&self, id: u64, cx: &Context<Self>) -> bool {
        (self.selected_session == Some(id) && !self.input.read(cx).value().trim().is_empty())
            || self
                .drafts
                .get(&id)
                .is_some_and(|draft| !draft.trim().is_empty())
            || self
                .composer_files
                .iter()
                .any(|(target, files)| target.session == Some(id) && !files.references.is_empty())
    }

    pub(super) fn delete_session(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.session_control.runs.contains_key(&id)
            || self.session_control.queue_saves.contains_key(&id)
            || self.session_draft_save_pending(id)
        {
            self.message =
                Some(conduit::i18n::text_static("sessions.a_task_is_still_running_in_this").into());
            self.delete_target = None;
            cx.notify();
            return;
        }
        if let Some((index, _)) = self.session_control.locate(&self.store.workspaces, id) {
            self.sidebar
                .invalidate_workspace(&self.store.workspaces[index].path);
        }
        for workspace in &mut self.store.workspaces {
            workspace.sessions.retain(|session| session.id != id);
        }
        self.drafts.remove(&id);
        self.queued_messages.remove(&id);
        self.session_control.unread_completed.remove(&id);
        self.expanded_errors.retain(|(session, _)| *session != id);
        self.composer_files
            .retain(|target, _| target.session != Some(id));
        self.expanded_tools.retain(|(session, _)| *session != id);
        self.expanded_tool_groups
            .retain(|(session, _)| *session != id);
        self.reasoning_expansion
            .retain(|(session, _), _| *session != id);
        self.tool_scrolls.retain(|(session, _), _| *session != id);
        self.tool_details.forget_session(id);
        self.tool_diffs.forget_session(id);
        self.remove_tab(id);
        self.select_fallback_session();
        self.delete_target = None;
        self.persist_and_cleanup_attachments();
        self.sync_git_workspace(cx);
        cx.notify();
    }

    pub(super) fn new_session_here(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let provider = self.new_session_provider;
        self.new_session(index, provider, window, cx);
    }

    pub(super) fn select_engine(
        &mut self,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || !self.store.cli.enabled(provider) {
            return;
        }
        if self.new_session_provider != provider {
            self.pending_model_selection = None;
            // An unsent conversation already owns its tab, draft and attachments.
            // Change its route in place while there is no engine context to resume.
            let target = self.selected_session.filter(|id| {
                !self.session_busy(*id)
                    && !self.session_control.queue_saves.contains_key(id)
                    && !self.session_draft_save_pending(*id)
                    && self.session(*id).is_some_and(|session| {
                        session.transcript.loaded
                            && session.messages.is_empty()
                            && session.native_id.is_none()
                            && session.exit_code.is_none()
                            && !session.interrupted
                    })
            });
            if let Some(id) = target {
                let selection = self.default_selection(provider);
                if let Some(session) = self.session_mut(id) {
                    session.provider = provider;
                    selection.apply(session);
                }
                self.persist();
            }
        }
        self.new_session_provider = provider;
        self.remember_provider(provider);
        if provider == Provider::OpenCode {
            self.ensure_opencode_models(false, cx);
        }
        // Conversations with history keep their engine and context. Switching
        // away from one configures the next new conversation without navigating.
        let _ = window;
        cx.notify();
    }

    /// Resolve the fallback against the model/channel that generated usage,
    /// once at turn start, rather than against the mutable next-turn picker.
    pub(super) fn usage_context_window_fallback(
        &self,
        provider: Provider,
        channel: &ModelChannel,
        model: Option<&str>,
    ) -> u64 {
        let models = if *channel == ModelChannel::Cli {
            self.managed_native_models(provider, true)
        } else {
            self.channel_models(provider, channel).1
        };
        let default_model = match channel {
            ModelChannel::Cli => self
                .engine_catalog(provider)
                .and_then(|c| c.current.as_deref()),
            _ => models.first().map(|m| m.id.as_str()),
        };
        model
            .or(default_model)
            .and_then(|id| {
                models
                    .iter()
                    .find(|m| m.id == id)
                    .and_then(|m| m.context_window)
                    .or_else(|| ModelCatalog::default().context_window(id))
            })
            .unwrap_or(0)
    }

    /// Sampled session-average output speed, provider consumption, and latest
    /// reported context. Unknown context never falls back to cumulative usage.
    pub(super) fn usage_status_parts(&self) -> Option<Vec<AnyElement>> {
        let usage = self
            .selected_session
            .and_then(|id| {
                self.session_control
                    .runs
                    .get(&id)
                    .map(|run| run.usage)
                    .or_else(|| self.session(id).map(|session| session.usage))
            })
            .unwrap_or_default();
        if usage.total_tokens == 0
            && usage.context_tokens.is_none()
            && usage.session_elapsed_ms == 0
        {
            return None;
        }
        let tokens_per_second = usage.average_output_speed();
        let format_tokens = |tokens: u64| {
            if tokens >= 1_000_000 {
                format!("{:.1}M", tokens as f64 / 1_000_000.0)
            } else if tokens >= 1_000 {
                format!("{:.1}K", tokens as f64 / 1_000.0)
            } else {
                format!("{tokens}")
            }
        };
        let context_ratio = usage.context_ratio();
        let context_window_display = usage.context_window;
        let estimated = usage.context_window_estimated;
        let context_ratio = context_ratio.unwrap_or(f64::INFINITY);
        let context_percent = (context_ratio * 100.) as f32;
        let context_color = if context_ratio >= 0.9 {
            palette().danger_text
        } else if context_ratio >= 0.75 {
            palette().warning_text
        } else {
            palette().highlight_text
        };
        let mut parts: Vec<AnyElement> = vec![];
        let speed_label = tokens_per_second
            .map(|speed| {
                conduit::i18n::format("sessions.avg_tok_s", &[("speed", format!("{}", speed))])
            })
            .unwrap_or_else(|| conduit::i18n::text_static("sessions.avg").into());
        let speed_tooltip = if usage.session_elapsed_ms > 0 {
            conduit::i18n::format(
                "sessions.average_session_output_speed_output_tokens_seconds",
                &[
                    ("arg0", format!("{}", usage.session_output_tokens)),
                    (
                        "arg1",
                        format!("{:.2}", usage.session_elapsed_ms as f64 / 1_000.0),
                    ),
                ],
            )
        } else {
            conduit::i18n::text_static("sessions.no_reliable_output_speed_measurements_for_this")
                .into()
        };
        parts.push(
            h_flex()
                .id("usage-speed")
                .gap_1()
                .items_center()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(speed_tooltip.clone())
                        .build(window, cx)
                })
                .child(
                    Icon::new(gpui_kit::assets::IconName::Zap)
                        .small()
                        .text_color(rgb(palette().faint)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(rgb(palette().muted))
                        .child(speed_label),
                )
                .into_any_element(),
        );
        parts.push(
            h_flex()
                .id("usage-total")
                .gap_1()
                .items_center()
                .child(
                    Icon::new(gpui_kit::assets::IconName::Gauge)
                        .small()
                        .text_color(rgb(palette().faint)),
                )
                .child(
                    div()
                        .text_size(ui_text_size(10.))
                        .text_color(rgb(palette().muted))
                        .child(format!(
                            "↑{} ↓{}",
                            format_tokens(usage.input_tokens),
                            format_tokens(usage.output_tokens)
                        )),
                )
                .into_any_element(),
        );
        if context_ratio.is_finite() {
            parts.push(
                h_flex()
                    .id("usage-context")
                    .tooltip(move |window, cx| {
                        let label = conduit::i18n::format(
                            "sessions.latest_cli_reported_context_tokens_this_is",
                            &[
                                ("arg0", format!("{}", usage.context_tokens.unwrap_or(0))),
                                ("arg1", format!("{}", usage.context_window)),
                                (
                                    "arg2",
                                    (if estimated {
                                        conduit::i18n::text_static(
                                            "sessions.window_estimated_from_this_turn_s_model",
                                        )
                                    } else {
                                        ""
                                    })
                                    .to_string(),
                                ),
                            ],
                        );
                        gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx)
                    })
                    .gap_1()
                    .items_center()
                    .child(
                        gpui_kit::component::progress::Progress::new("context-progress")
                            .value(context_percent)
                            .color(rgb(context_color))
                            .xsmall()
                            .w(px(40.))
                            .flex_shrink_0()
                            .accessibility_label(conduit::i18n::format(
                                "sessions.context_usage",
                                &[("arg0", format!("{:.0}", context_percent))],
                            )),
                    )
                    .child(
                        div()
                            .text_size(ui_text_size(10.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{}{} / {}",
                                if estimated { "≈" } else { "" },
                                format_tokens(usage.context_tokens.unwrap_or(0)),
                                format_tokens(context_window_display)
                            )),
                    )
                    .into_any_element(),
            );
        }
        if parts.is_empty() {
            return None;
        }
        Some(parts)
    }

    pub(super) fn open_remove_workspace_dialog(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .store
            .workspaces
            .get(index)
            .map(|workspace| workspace.name())
        else {
            return;
        };
        self.remove_workspace_target = Some(index);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let name = name.clone();
            dialog
                .w(px(440.))
                .margin_top(px(120.))
                .title(conduit::i18n::text_static("sessions.remove_workspace"))
                .content(move |content, _, _| {
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(ui_text_size(13.))
                                    .line_height(relative(1.5))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::format(
                                        "sessions.remove_from_conduit_files_on_disk_and",
                                        &[("name", name.to_string())],
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("remove-workspace-cancel")
                                            .label(conduit::i18n::text_static(
                                                "git_branches.cancel",
                                            ))
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                cancel.update(cx, |this, cx| {
                                                    this.remove_workspace_target = None;
                                                    cx.notify();
                                                });
                                                window.close_dialog(cx);
                                            }),
                                    )
                                    .child(
                                        Button::new("remove-workspace-confirm")
                                            .label(conduit::i18n::text_static("sessions.remove"))
                                            .small()
                                            .primary()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                let target =
                                                    confirm.read(cx).remove_workspace_target;
                                                if let Some(index) = target {
                                                    confirm.update(cx, |this, cx| {
                                                        this.remove_workspace(index, cx)
                                                    });
                                                }
                                                window.close_dialog(cx);
                                            }),
                                    ),
                            ),
                    )
                })
        });
    }

    pub(super) fn remove_workspace(&mut self, index: usize, cx: &mut Context<Self>) {
        self.remove_workspace_target = None;
        if index >= self.store.workspaces.len() {
            return;
        }
        if self.workspace_busy(index) {
            self.message = Some(
                conduit::i18n::text_static("sessions.tasks_or_terminals_are_still_running_in")
                    .into(),
            );
            cx.notify();
            return;
        }
        let removed = self.store.workspaces.remove(index);
        self.terminals.forget_workspace(&removed.path);
        self.history_loading.remove(&removed.path);
        self.collapsed_workspaces.remove(&removed.path);
        self.workspace_drafts.remove(&removed.path);
        self.git.selected_files.remove(&removed.path);
        self.composer_files
            .retain(|target, _| target.workspace != removed.path);
        self.expanded_workspaces.clear();
        for session in &removed.sessions {
            self.drafts.remove(&session.id);
            self.queued_messages.remove(&session.id);
            self.session_control.unread_completed.remove(&session.id);
            self.expanded_errors.retain(|(id, _)| *id != session.id);
            self.expanded_tools.retain(|(id, _)| *id != session.id);
            self.reasoning_expansion
                .retain(|(id, _), _| *id != session.id);
            self.tool_scrolls.retain(|(id, _), _| *id != session.id);
            self.tool_details.forget_session(session.id);
            self.tool_diffs.forget_session(session.id);
        }
        let removed_ids: Vec<u64> = removed.sessions.iter().map(|session| session.id).collect();
        self.open_tabs
            .retain(|tab| tab.chat().is_none_or(|id| !removed_ids.contains(&id)));
        self.forget_workspace_files(&removed.path);
        if self.selected_workspace == Some(index) || self.selected_session.is_none() {
            self.selected_workspace = None;
            self.selected_session = None;
            self.select_fallback_session();
        } else if let Some(selected) = self.selected_workspace
            && selected > index
        {
            self.selected_workspace = Some(selected - 1);
        }
        self.persist_and_cleanup_attachments();
        self.sync_git_workspace(cx);
        cx.notify();
    }

    pub(super) fn workspace_menu(&self, index: usize, app: Entity<Self>) -> AnyElement {
        let selected = self.selected_workspace == Some(index);
        let trigger = Button::new(("workspace-menu-trigger", index))
            .ghost()
            .small()
            .compact()
            .w(px(22.))
            .h(px(22.))
            .p_0()
            .opacity(0.)
            .group_hover("workspace-row", |style| style.opacity(1.))
            .focus_visible(|style| style.opacity(1.))
            .icon(IconName::Ellipsis)
            .cursor_pointer()
            .on_click(|_, _, cx| cx.stop_propagation());
        Popover::new(("workspace-menu", index))
            .anchor(Anchor::LeftCenter)
            .offset(px(8.))
            .refine_style(&menu_popover_style())
            .trigger(trigger)
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                let clear_app = app.clone();
                let remove_app = app.clone();
                let clear_popover = popover.clone();
                let remove_popover = popover.clone();
                let clear = session_menu_button(
                    ("workspace-clear-selection", index as u64),
                    conduit::i18n::text_static("sessions.deselect_workspace"),
                    IconName::CircleX,
                )
                .on_click(move |_, window, cx| {
                    clear_app.update(cx, |this, cx| this.clear_workspace_selection(window, cx));
                    if let Some(popover) = clear_popover.clone().upgrade() {
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    }
                });
                let remove = session_menu_button(
                    ("workspace-remove", index as u64),
                    conduit::i18n::text_static("sessions.remove_workspace"),
                    IconName::Delete,
                )
                .on_click(move |_, window, cx| {
                    remove_app.update(cx, |this, cx| {
                        this.open_remove_workspace_dialog(index, window, cx)
                    });
                    if let Some(popover) = remove_popover.clone().upgrade() {
                        popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                    }
                });
                let mut content = v_flex().w(px(170.)).gap(px(1.));
                if selected {
                    content = content.child(clear);
                }
                content.child(remove)
            })
            .into_any_element()
    }

    /// Toggle the shared sidebar, retaining the last available tab.
    pub(super) fn detail_toggle(&self, cx: &mut Context<Self>) -> Button {
        let open = self.plugins.right_visible(self.branch_navigation_open());
        Button::new("toggle-sidebar")
            .debug_selector(|| "toggle-sidebar".into())
            .ghost()
            .small()
            .compact()
            .icon(gpui_kit::assets::IconName::PanelRight)
            .cursor_pointer()
            .selected(open)
            .tooltip(if open {
                conduit::i18n::text_static("sessions.hide_sidebar")
            } else {
                conduit::i18n::text_static("sessions.show_sidebar")
            })
            .accessibility_label(if open {
                conduit::i18n::text_static("sessions.hide_sidebar")
            } else {
                conduit::i18n::text_static("sessions.show_sidebar")
            })
            .on_click(cx.listener(|this, _, window, cx| {
                if this.plugins.right_visible(this.branch_navigation_open()) {
                    this.close_right_view(window, cx);
                } else {
                    let selected = this.plugins.right().clone();
                    if !this.open_right_view(&selected, window, cx) {
                        this.open_right_view(
                            &ViewId::new(builtins::NOTIFICATIONS).unwrap(),
                            window,
                            cx,
                        );
                    }
                }
            }))
    }

    pub(super) fn clear_workspace_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        } else if let Some(target) = self.composer_target() {
            self.workspace_drafts
                .insert(target.workspace, self.input.read(cx).value().to_string());
        }
        self.active_file = None;
        self.active_diff = None;
        self.selected_workspace = None;
        self.selected_session = None;
        self.message = None;
        self.message_focus = None;
        self.pending_message_jump = None;
        self.timeline_session = None;
        self.input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.sync_git_workspace(cx);
        cx.notify();
    }

    pub(super) fn session_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        self.document_tab_bar(cx)
    }

    pub(super) fn switch(
        &mut self,
        workspace: usize,
        session: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_branch_navigation();
        self.switch_context(workspace, session, true, window, cx);
    }

    pub(super) fn switch_context(
        &mut self,
        workspace: usize,
        session: Option<u64>,
        activate_chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if session.is_some_and(|id| self.pending_new_session(id)) {
            self.message = Some(
                conduit::i18n::text_static("sessions.saving_new_session_please_wait_your_draft")
                    .into(),
            );
            cx.notify();
            return;
        }
        if let Some(id) = self.selected_session
            && self.timeline_session == Some(id)
        {
            let top = self.scroll.logical_scroll_top();
            if let Some(row) = self.chat_rows.rows.get(top.item_ix) {
                self.reading_positions
                    .insert(id, (row.key.clone(), top.offset_in_item, self.at_bottom));
            } else if self.at_bottom {
                self.reading_positions
                    .insert(id, (String::new(), px(0.), true));
            }
        }
        if let Some(id) = self.selected_session {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
        } else if let Some(target) = self.composer_target() {
            self.workspace_drafts
                .insert(target.workspace, self.input.read(cx).value().to_string());
        }
        self.active_file = None;
        self.active_diff = None;
        self.selected_workspace = Some(workspace);
        self.selected_session = session;
        self.ensure_selected_transcript(cx);
        self.trim_transcripts();
        if activate_chat && let Some(id) = session {
            self.store.ui.recent_sessions.retain(|recent| *recent != id);
            self.store.ui.recent_sessions.insert(0, id);
            self.store.ui.recent_sessions.truncate(10);
            self.dirty = true;
        }
        self.pending_model_selection = None;
        if activate_chat && let Some(id) = session {
            self.session_control.unread_completed.remove(&id);
            self.push_tab(id);
        }
        self.message_focus = None;
        self.pending_message_jump = None;
        self.message = None;
        let draft = match session {
            Some(id) => self.drafts.get(&id),
            None => self
                .workspace_drafts
                .get(&self.store.workspaces[workspace].path),
        }
        .cloned()
        .unwrap_or_default();
        self.input
            .update(cx, |input, cx| input.set_value(draft, window, cx));
        // The recalled-prompt rank belongs to the conversation we just left.
        self.prompt_history = None;
        if activate_chat && session.is_some() {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.timeline_session = None;
        self.at_bottom = session
            .and_then(|id| self.reading_positions.get(&id))
            .is_none_or(|position| position.2);
        self.restore_reading_position = !self.at_bottom;
        if activate_chat
            && let Some(provider) = session
                .and_then(|id| self.session(id))
                .map(|session| session.provider)
        {
            self.new_session_provider = provider;
            self.remember_provider(provider);
        }
        // Branch metadata and diffs always belong to the selected workspace.
        self.sync_git_workspace(cx);
        cx.notify();
    }

    pub(super) fn add_workspace(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(
                conduit::i18n::text_static("sessions.select_a_workspace_directory_you_trust")
                    .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| match paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        match path.canonicalize() {
                            Ok(path) if path.is_dir() => {
                                let index = this
                                    .store
                                    .workspaces
                                    .iter()
                                    .position(|w| w.path == path)
                                    .unwrap_or_else(|| {
                                        this.store.workspaces.push(Workspace {
                                            path,
                                            sessions: vec![],
                                        });
                                        this.store.workspaces.len() - 1
                                    });
                                let session = this.store.workspaces[index]
                                    .sessions
                                    .iter()
                                    .find(|session| !session.archived)
                                    .map(|session| session.id);
                                this.switch(index, session, window, cx);
                                this.persist();
                                this.start_history_import(index, window, cx);
                            }
                            _ => {
                                this.message = Some(
                                    conduit::i18n::text_static(
                                        "sessions.directory_does_not_exist_or_is_inaccessible",
                                    )
                                    .into(),
                                )
                            }
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.message = Some(conduit::i18n::format(
                        "sessions.failed_to_open_directory",
                        &[("other", format!("{:?}", other))],
                    ));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Scans the workspace's CLI history off the UI thread. The sidebar shows a
    /// loading state until [`Self::finish_history_import`] merges the result.
    pub(super) fn start_history_import(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.store.workspaces.get(index) else {
            return;
        };
        let path = workspace.path.clone();
        if !self.history_loading.insert(path.clone()) {
            return;
        }
        self.session_control.begin_import(workspace);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.session_control
            .scan_cancels
            .insert(path.clone(), cancel.clone());
        self.run_history_import(vec![path], cancel, true, window, cx);
    }

    /// Imports CLI history for every known workspace once after startup, off
    /// the UI thread. Unlike a user-triggered import this refreshes the
    /// in-memory transcripts only; the next ordinary save persists them.
    pub(crate) fn start_initial_history_imports(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<PathBuf> = self
            .store
            .workspaces
            .iter()
            .map(|workspace| workspace.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        for workspace in &self.store.workspaces {
            self.session_control
                .scan_cancels
                .insert(workspace.path.clone(), cancel.clone());
            self.session_control.begin_import(workspace);
            self.history_loading.insert(workspace.path.clone());
        }
        cx.notify();
        self.run_history_import(paths, cancel, false, window, cx);
    }

    fn run_history_import(
        &mut self,
        paths: Vec<PathBuf>,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        persist: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let scan_paths = paths.clone();
            let ticket = cancel.clone();
            let mut scanner = cx
                .background_executor()
                .spawn(async move { crate::history::Scanner::new(scan_paths, &ticket) })
                .await;
            loop {
                let ticket = cancel.clone();
                let (next_scanner, batch) = cx
                    .background_executor()
                    .spawn(async move {
                        let batch = scanner.next(&ticket);
                        (scanner, batch)
                    })
                    .await;
                scanner = next_scanner;
                let done = batch.done;
                let Ok(workspaces) = this.update(cx, |this, _| {
                    paths
                        .iter()
                        .zip(&batch.sessions)
                        .map(|(path, incoming)| {
                            let keys: HashSet<_> = incoming
                                .iter()
                                .map(|session| (session.provider, session.native_id.as_str()))
                                .collect();
                            let sessions = this
                                .store
                                .workspaces
                                .iter()
                                .find(|workspace| &workspace.path == path)
                                .into_iter()
                                .flat_map(|workspace| &workspace.sessions)
                                .filter(|session| {
                                    session.native_id.as_deref().is_some_and(|native| {
                                        keys.contains(&(session.provider, native))
                                    })
                                })
                                .cloned()
                                .collect();
                            Workspace {
                                path: path.clone(),
                                sessions,
                            }
                        })
                        .collect::<Vec<_>>()
                }) else {
                    break;
                };
                let incoming = batch.sessions;
                let ticket = cancel.clone();
                let (prepared, warnings) = cx
                    .background_executor()
                    .spawn(async move {
                        let mut prepared = Vec::new();
                        let mut warnings = batch.warnings;
                        let directory = crate::transcript::directory(&Store::path());
                        for (workspace, incoming) in workspaces.into_iter().zip(incoming) {
                            if ticket.load(std::sync::atomic::Ordering::Acquire) {
                                break;
                            }
                            let (records, errors) =
                                crate::history::prepare_imported(&workspace, incoming, &directory);
                            prepared.push(records);
                            warnings.extend(errors);
                        }
                        (prepared, warnings)
                    })
                    .await;
                let updated = this.update_in(cx, |this, window, cx| {
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return false;
                    }
                    if !warnings.is_empty() {
                        this.message = Some(warnings.join("\n"));
                    }
                    for (path, sessions) in paths.iter().zip(prepared) {
                        if !this
                            .session_control
                            .scan_cancels
                            .get(path)
                            .is_some_and(|ticket| std::sync::Arc::ptr_eq(ticket, &cancel))
                        {
                            continue;
                        }
                        if !sessions.is_empty() || done {
                            this.finish_prepared_history_import(
                                path,
                                sessions,
                                persist && done,
                                done,
                                window,
                                cx,
                            );
                        }
                        if done {
                            this.session_control.scan_cancels.remove(path);
                            this.session_control.scan_progress.remove(path);
                        } else {
                            this.history_loading.insert(path.clone());
                            this.session_control
                                .scan_progress
                                .insert(path.clone(), (batch.completed, batch.total));
                        }
                    }
                    if done && persist && this.dirty {
                        this.persist();
                    }
                    if let Some(startup) = &mut this.startup {
                        startup.imports_done = true;
                    }
                    cx.notify();
                    true
                });
                if done || !matches!(updated, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(super) fn finish_history_import(
        &mut self,
        path: &Path,
        imported: Vec<crate::history::ImportedSession>,
        persist: bool,
        complete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .store
            .workspaces
            .iter()
            .find(|workspace| workspace.path == path)
        else {
            return;
        };
        let (records, warnings) = crate::history::prepare_imported(
            workspace,
            imported,
            &crate::transcript::directory(&Store::path()),
        );
        if !warnings.is_empty() {
            self.message = Some(warnings.join("\n"));
        }
        self.finish_prepared_history_import(path, records, persist, complete, window, cx);
    }

    fn finish_prepared_history_import(
        &mut self,
        path: &Path,
        imported: Vec<SessionRecord>,
        persist: bool,
        complete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if complete {
            self.history_loading.remove(path);
        }
        // The workspace may have been removed while the scan was running.
        let Some(index) = self
            .store
            .workspaces
            .iter()
            .position(|workspace| workspace.path == path)
        else {
            return;
        };
        // A scan can finish after a turn has started. Its older transcript must
        // never replace messages or invalidate the active run's message index.
        let versions = self.session_control.imports.get(path);
        let imported = imported
            .into_iter()
            .filter(|imported| {
                !self.store.workspaces[index].sessions.iter().any(|session| {
                    session.provider == imported.provider
                        && session.native_id == imported.native_id
                        && (self.session_control.runs.contains_key(&session.id)
                            || versions.is_some_and(|versions| {
                                !self.session_control.unchanged(versions, session.id)
                            }))
                })
            })
            .collect::<Vec<_>>();
        if complete {
            self.session_control.imports.remove(path);
        }
        let discovered = !imported.is_empty();
        if discovered {
            self.sidebar.invalidate_workspace(path);
            self.session_control.global_revision =
                self.session_control.global_revision.wrapping_add(1);
            // Native import can replace keys without changing message count.
            self.outline = MessageOutlineCache::default();
            self.timeline_session = None;
            self.message_focus = None;
            self.pending_message_jump = None;
        }
        for mut record in imported {
            if let Some(existing) =
                self.store.workspaces[index]
                    .sessions
                    .iter_mut()
                    .find(|session| {
                        session.provider == record.provider && session.native_id == record.native_id
                    })
            {
                // A deleted/recreated native conversation is a different identity.
                if record.id != existing.id {
                    continue;
                }
                record.pinned = existing.pinned;
                *existing = record;
            } else if record.id == 0 {
                record.id = self.next_id;
                self.next_id += 1;
                self.store.workspaces[index].sessions.push(record);
            }
        }
        self.store.workspaces[index]
            .sessions
            .sort_by_key(|session| std::cmp::Reverse((session.last_activity_at(), session.id)));
        self.ensure_selected_transcript(cx);
        self.trim_transcripts();
        // Open the newest session on the workspace the user just added, unless
        // they already picked one or started typing while history loaded.
        let follow = self.selected_workspace == Some(index)
            && self.selected_session.is_none()
            && self.input.read(cx).value().trim().is_empty()
            && self
                .current_files()
                .is_none_or(|files| files.references.is_empty());
        if follow
            && let Some(session) = self.store.workspaces[index]
                .sessions
                .iter()
                .find(|session| !session.archived)
                .map(|session| session.id)
        {
            self.switch(index, Some(session), window, cx);
        }
        if discovered {
            self.dirty = true;
        }
        if discovered && persist {
            self.persist();
        }
        cx.notify();
    }

    pub(super) fn new_session(
        &mut self,
        index: usize,
        provider: Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_workspace = self.selected_workspace;
        let previous_session = self.selected_session;
        if let Some(id) = self.create_session(index, provider, cx) {
            // create_session selects its result; switch must first save the draft
            // from the conversation the user was actually editing.
            self.selected_workspace = previous_workspace;
            self.selected_session = previous_session;
            self.switch(index, Some(id), window, cx);
        }
    }

    /// Append a session without touching the composer draft. Used by the "+"
    /// buttons (which then switch to it) and by "type to start" (which sends
    /// the current prompt immediately).
    pub(super) fn create_session(
        &mut self,
        index: usize,
        provider: Provider,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        if !self.writable || self.closing || index >= self.store.workspaces.len() {
            return None;
        }
        let session = self.prepare_session(provider);
        let id = session.id;
        self.store.workspaces[index].sessions.insert(0, session);
        self.select_created_session(index, id, provider);
        self.persist();
        cx.notify();
        Some(id)
    }

    /// Build the exact selection for validation without creating an empty session
    /// or consuming the composer's pending model selection.
    pub(super) fn prepare_session(&mut self, provider: Provider) -> SessionRecord {
        let id = self.next_id;
        if provider != Provider::OpenCode {
            self.catalogs.insert(provider, models::load(provider));
        }
        self.ccs = ccs::load();
        self.store.sync_ccs_providers(&self.ccs);
        let model = self
            .catalogs
            .get(&provider)
            .and_then(|catalog| catalog.current.clone());
        let mut session = SessionRecord {
            transcript: Default::default(),
            id,
            provider,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            exit_code: None,
            title: conduit::i18n::text_static("files.new_session").into(),
            archived: false,
            pinned: false,
            native_id: None,
            model,
            messages: vec![],
            interrupted: false,
            channel: Default::default(),
            reasoning_effort: ReasoningEffort::for_provider(provider),
            opencode: Default::default(),
            permission: self.pending_permission,
            usage: Usage::default(),
        };
        if self.store.session_defaults.contains_key(&provider) {
            self.default_selection(provider).apply(&mut session);
        }
        if let Some(selection) = self
            .pending_model_selection
            .as_ref()
            .filter(|s| s.engine == provider)
        {
            selection.clone().apply(&mut session);
        }
        session
    }

    pub(super) fn select_created_session(&mut self, index: usize, id: u64, provider: Provider) {
        self.next_id = self.next_id.max(id + 1);
        self.collapsed_workspaces
            .remove(&self.store.workspaces[index].path);
        self.selected_workspace = Some(index);
        self.active_file = None;
        self.active_diff = None;
        self.selected_session = Some(id);
        if self
            .pending_model_selection
            .as_ref()
            .is_some_and(|s| s.engine == provider)
        {
            self.pending_model_selection = None;
        }
        self.sidebar
            .invalidate_workspace(&self.store.workspaces[index].path);
        self.push_tab(id);
        self.new_session_provider = provider;
        self.store.ui.last_provider = Some(provider);
    }

    pub(super) fn remember_provider(&mut self, provider: Provider) {
        if self.store.ui.last_provider != Some(provider) {
            self.store.ui.last_provider = Some(provider);
            self.persist();
        }
    }

    /// Daily session counts across every workspace for the last `weeks * 7`
    /// days, anchored to today's local date. Buckets use the local timezone so
    /// a session made just after midnight lands on the same day the user sees.
    pub(super) fn session_activity(&self, weeks: usize) -> (chrono::NaiveDate, Vec<u32>) {
        let today = Local::now().date_naive();
        let days = weeks * 7;
        let start_day = today - chrono::Duration::days((days - 1) as i64);
        let mut counts = vec![0u32; days];
        for workspace in &self.store.workspaces {
            for session in &workspace.sessions {
                if session.created_at == 0 {
                    continue;
                }
                let Some(created) = DateTime::from_timestamp(session.created_at as i64, 0) else {
                    continue;
                };
                let day = created.with_timezone(&Local).date_naive();
                let offset = (day - start_day).num_days();
                if (0..days as i64).contains(&offset) {
                    counts[offset as usize] += 1;
                }
            }
        }
        (today, counts)
    }

    pub(super) fn heatmap_cell_color(count: u32, max: u32) -> u32 {
        if count == 0 || max == 0 {
            return palette().surface;
        }
        let base = palette().surface_hover;
        let accent = palette().diff_add_text;
        let ratio = 0.25 + 0.75 * (count as f32 / max as f32);
        let mix = |a: u32, b: u32, t: f32| -> u8 { (a as f32 + (b as f32 - a as f32) * t) as u8 };
        let r = mix(base >> 16 & 0xff, accent >> 16 & 0xff, ratio);
        let g = mix(base >> 8 & 0xff, accent >> 8 & 0xff, ratio);
        let b = mix(base & 0xff, accent & 0xff, ratio);
        ((r as u32) << 16) | ((g as u32) << 8) | b as u32
    }

    /// Activity heatmap shown in place of the chat while no workspace is
    /// selected: 7 rows (weekdays) by trailing weeks, newest column last.
    pub(super) fn session_heatmap(&self, weeks: usize, cx: &mut Context<Self>) -> AnyElement {
        const WEEKDAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
        let (today, counts) = self.session_activity(weeks);
        let start_day = today - chrono::Duration::days((weeks * 7 - 1) as i64);
        let total: u32 = counts.iter().sum();
        let max = counts.iter().copied().max().unwrap_or(0);

        let mut grid = h_flex().gap(px(3.));
        for week in 0..weeks {
            let mut column = v_flex().gap(px(3.));
            for weekday in 0..7 {
                let index = week * 7 + weekday;
                let count = counts[index];
                let day = start_day + chrono::Duration::days(index as i64);
                let label: SharedString = conduit::i18n::format(
                    "sessions.sessions",
                    &[
                        ("arg0", format!("{}", day.format("%Y-%m-%d"))),
                        ("arg1", format!("{}", count)),
                    ],
                )
                .into();
                column = column.child(
                    div()
                        .id(("heatmap-cell", index))
                        .w(px(24.))
                        .h(px(12.))
                        .rounded(px(3.))
                        .bg(rgb(Self::heatmap_cell_color(count, max)))
                        .border_1()
                        .border_color(rgb(palette().border))
                        .when(day > today, |cell| cell.opacity(0.25))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        }),
                );
            }
            grid = grid.child(column);
        }

        let mut weekday_labels = v_flex().gap(px(3.)).pt(px(0.));
        for weekday in WEEKDAYS {
            weekday_labels = weekday_labels.child(
                div()
                    .h(px(12.))
                    .text_size(ui_text_size(9.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::source(weekday)),
            );
        }

        v_flex()
            .flex_1()
            .items_center()
            .justify_center()
            .gap_5()
            .py_20()
            .child(
                div()
                    .text_size(ui_text_size(30.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child(conduit::i18n::text_static(
                        "sessions.connect_a_local_workspace",
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("welcome-add-workspace")
                            .label(conduit::i18n::text_static("sessions.add_workspace"))
                            .primary()
                            .disabled(!self.writable || self.closing)
                            .on_click(cx.listener(Self::add_workspace)),
                    )
                    .child(
                        Button::new("welcome-engine-settings")
                            .label(conduit::i18n::text_static("sessions.check_ai_engines"))
                            .secondary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_engine_settings(window, cx)
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::format(
                                "sessions.sessions_in_the_last_weeks",
                                &[
                                    ("weeks", format!("{}", weeks)),
                                    ("total", format!("{}", total)),
                                ],
                            )),
                    )
                    .child(h_flex().gap_2().child(weekday_labels).child(grid)),
            )
            .child(
                div()
                    .text_size(ui_text_size(11.))
                    .text_color(rgb(palette().faint))
                    .child(conduit::i18n::text_static(
                        "sessions.the_selected_directory_is_used_as_the",
                    )),
            )
            .into_any_element()
    }
}
