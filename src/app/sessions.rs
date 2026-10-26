//! Workspace/session lifecycle, history import, and navigation.

use super::*;

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
        NamingMode::Latest => "根据最新内容",
        NamingMode::FirstMessage => "根据第一条消息",
    }
}

/// Fit a naming failure into the toolbar's single line: first non-empty line,
/// capped at a readable length.
pub(super) fn naming_failure_text(error: &str) -> String {
    let line = error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("未知错误");
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(120).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Whether a session matches the sidebar search query (title or message text).
pub(super) fn session_matches(session: &SessionRecord, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    session.title.to_lowercase().contains(query)
        || session
            .messages
            .iter()
            .any(|message| message.text.to_lowercase().contains(query))
}

impl ConduitApp {
    pub(super) fn session(&self, id: u64) -> Option<&SessionRecord> {
        self.store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|s| s.id == id)
    }

    pub(super) fn session_mut(&mut self, id: u64) -> Option<&mut SessionRecord> {
        self.store
            .workspaces
            .iter_mut()
            .flat_map(|w| &mut w.sessions)
            .find(|s| s.id == id)
    }

    pub(super) fn select_fallback_session(&mut self) {
        if self
            .selected_session
            .is_some_and(|id| self.session(id).is_some_and(|session| !session.archived))
        {
            self.push_tab(self.selected_session.unwrap());
            self.unread_completed
                .remove(&self.selected_session.unwrap());
            return;
        }
        for (index, workspace) in self.store.workspaces.iter().enumerate() {
            if let Some(session) = workspace.sessions.iter().find(|s| !s.archived) {
                self.selected_workspace = Some(index);
                self.selected_session = Some(session.id);
                self.unread_completed.remove(&session.id);
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

    /// Open a tab without reordering the strip. New tabs append at the end and
    /// the oldest non-selected tab is evicted past the configured limit.
    pub(super) fn push_tab(&mut self, id: u64) {
        if !self.open_tabs.contains(&id) {
            self.open_tabs.push(id);
        }
        let limit = self.store.ui.max_tabs.clamp(1, MAX_TAB_LIMIT);
        while self.open_tabs.len() > limit {
            if let Some(position) = self
                .open_tabs
                .iter()
                .position(|tab| Some(*tab) != self.selected_session)
            {
                self.open_tabs.remove(position);
            } else {
                break;
            }
        }
    }

    /// Drop a tab without touching the current selection; used when the tab
    /// is closed while another session stays active.
    pub(super) fn remove_tab(&mut self, id: u64) {
        self.open_tabs.retain(|tab| *tab != id);
    }

    /// Close a group of tabs while keeping the conversation records in the
    /// sidebar. If the active tab is among them, select the newest remaining
    /// open tab, matching the single-tab close behavior.
    pub(super) fn close_tabs(&mut self, ids: &[u64], cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let closing_active = self.selected_session.is_some_and(|id| ids.contains(&id));
        if closing_active {
            if let Some(id) = self.selected_session {
                self.drafts
                    .insert(id, self.input.read(cx).value().to_string());
            }
        }
        self.open_tabs.retain(|tab| !ids.contains(tab));
        if !closing_active {
            cx.notify();
            return;
        }

        let next = self
            .open_tabs
            .iter()
            .rev()
            .copied()
            .find(|id| self.session(*id).is_some());
        if let Some(next) = next {
            let window = cx
                .windows()
                .into_iter()
                .find_map(|handle| handle.downcast::<Self>());
            if let Some(window) = window {
                let _ = window.update(cx, |_, window, cx| {
                    if let Some(workspace) = self.store.workspaces.iter().position(|workspace| {
                        workspace.sessions.iter().any(|session| session.id == next)
                    }) {
                        self.switch(workspace, Some(next), window, cx);
                    }
                });
            }
        } else {
            self.selected_session = None;
            self.detail_text = None;
            self.message = None;
            cx.notify();
        }
    }

    pub(super) fn close_tabs_left(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(position) = self.open_tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        let ids = self.open_tabs[..position].to_vec();
        self.close_tabs(&ids, cx);
    }

    pub(super) fn close_tabs_right(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(position) = self.open_tabs.iter().position(|tab| *tab == id) else {
            return;
        };
        let ids = self.open_tabs[position + 1..].to_vec();
        self.close_tabs(&ids, cx);
    }

    pub(super) fn close_all_tabs(&mut self, cx: &mut Context<Self>) {
        let ids = self.open_tabs.clone();
        self.close_tabs(&ids, cx);
    }

    pub(super) fn toggle_archive(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(session) = self.session_mut(id) {
            session.archived = !session.archived;
        }
        let archived_tabs: Vec<u64> = self
            .open_tabs
            .iter()
            .copied()
            .filter(|tab| self.session(*tab).is_none_or(|session| session.archived))
            .collect();
        self.open_tabs.retain(|tab| !archived_tabs.contains(tab));
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
                .title("重命名会话")
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
                                        .label("取消")
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
                                        .label("保存")
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
            self.message = Some("会话名不能为空。".into());
            cx.notify();
            return;
        }
        // A manual rename wins over any title the naming engine still owes us.
        if let Some(naming) = self.session_namings.remove(&id) {
            naming.runtime.cancel();
        }
        self.naming_errors.remove(&id);
        if let Some(session) = self.session_mut(id) {
            session.title = title.chars().take(80).collect();
        }
        self.persist();
        cx.notify();
    }

    pub(super) fn open_delete_dialog(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(id) else {
            return;
        };
        let title = if session.title.is_empty() {
            "未命名会话".to_string()
        } else {
            session.title.clone()
        };
        // A session that never received a message has no history to protect, so
        // its delete acts immediately. Unsent composer text still asks first.
        if session.messages.is_empty() && !self.session_has_unsent_content(id, cx) {
            self.delete_session(id, cx);
            return;
        }
        self.delete_target = Some(id);
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            let title = title.clone();
            dialog
                .w(px(420.))
                .margin_top(px(120.))
                .title("删除会话")
                .content(move |content, _, _| {
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(relative(1.5))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "将永久删除「{title}」及其消息记录，无法恢复。"
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("delete-cancel")
                                            .label("取消")
                                            .small()
                                            .ghost()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                cancel.update(cx, |this, cx| {
                                                    this.delete_target = None;
                                                    cx.notify();
                                                });
                                                window.close_dialog(cx);
                                            }),
                                    )
                                    .child(
                                        Button::new("delete-confirm")
                                            .label("删除")
                                            .small()
                                            .primary()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                let target = confirm.read(cx).delete_target;
                                                if let Some(id) = target {
                                                    confirm.update(cx, |this, cx| {
                                                        this.delete_session(id, cx)
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
        if self.runs.contains_key(&id) {
            self.message = Some("该会话仍有任务运行，请先取消后再删除。".into());
            self.delete_target = None;
            cx.notify();
            return;
        }
        for workspace in &mut self.store.workspaces {
            workspace.sessions.retain(|session| session.id != id);
        }
        self.drafts.remove(&id);
        self.unread_completed.remove(&id);
        self.expanded_errors.retain(|(session, _)| *session != id);
        self.composer_files
            .retain(|target, _| target.session != Some(id));
        self.expanded_tools.retain(|(session, _)| *session != id);
        self.expanded_tool_groups
            .retain(|(session, _)| *session != id);
        self.expanded_reasoning
            .retain(|(session, _)| *session != id);
        self.tool_scrolls.retain(|(session, _), _| *session != id);
        self.remove_tab(id);
        self.detail_text = None;
        self.select_fallback_session();
        self.delete_target = None;
        self.persist();
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
        self.new_session_provider = provider;
        self.remember_provider(provider);
        // Switching the engine targets the *next* new session: it never
        // jumps to an existing history session. The current session stays
        // selected and its tab stays open.
        let _ = window;
        cx.notify();
    }

    /// Compact token/usage indicators (tokens/s while streaming, cumulative
    /// usage, and a ring gauge for context window consumption) rendered on
    /// the composer's engine row. Reads from the active run when streaming
    /// and falls back to the session's last persisted snapshot when idle.
    /// Returns `None` when there is nothing to show yet.
    pub(super) fn usage_status_parts(&self) -> Option<Vec<AnyElement>> {
        let session = self.selected_session.and_then(|id| self.session(id));
        let (usage, streaming, elapsed, since_event) = match self.selected_session {
            Some(id) if self.runs.contains_key(&id) => {
                let run = &self.runs[&id];
                (
                    run.usage,
                    true,
                    run.turn_started.elapsed(),
                    run.last_usage_event.elapsed(),
                )
            }
            _ => (
                session.map(|s| s.usage).unwrap_or_default(),
                false,
                Duration::ZERO,
                Duration::ZERO,
            ),
        };
        if usage.total_tokens == 0 {
            return None;
        }
        // Tokens/s is derived from this turn's delta; pause the clock while no
        // usage events arrive (approval wait, model idle) so the figure does
        // not decay toward zero during stalls.
        let run = self.selected_session.and_then(|id| self.runs.get(&id));
        let tokens_per_second = if streaming {
            let baseline = run
                .map(|r| r.turn_tokens_at_start)
                .unwrap_or(usage.total_tokens);
            let active = if since_event < Duration::from_secs(10) {
                elapsed
            } else {
                Duration::ZERO
            };
            let seconds = active.as_secs_f64();
            if seconds >= 1.0 && usage.total_tokens > baseline {
                Some(((usage.total_tokens - baseline) as f64 / seconds).round() as u64)
            } else {
                None
            }
        } else {
            None
        };
        let format_tokens = |tokens: u64| {
            if tokens >= 1_000_000 {
                format!("{:.1}M", tokens as f64 / 1_000_000.0)
            } else if tokens >= 1_000 {
                format!("{:.1}K", tokens as f64 / 1_000.0)
            } else {
                format!("{tokens}")
            }
        };
        let context_ratio = {
            // The CLI-reported window (Codex) wins; Claude stream-json carries
            // no window size, so fall back to the model catalog (context_window
            // field or the model ID's `[1M]` suffix). Estimated rings show a
            // "≈" prefix since the real window is model-side policy.
            let window = if usage.context_window > 0 {
                Some(usage.context_window)
            } else {
                let provider = session
                    .map(|s| s.provider)
                    .unwrap_or(self.new_session_provider);
                let channel = session.map(|s| s.channel.clone()).unwrap_or_default();
                let (_, models) = self.channel_models(provider, &channel);
                let default_model = match &channel {
                    ModelChannel::Cli => {
                        self.catalogs.get(&provider).and_then(|c| c.current.clone())
                    }
                    ModelChannel::Ccs(_) | ModelChannel::Provider(_) => {
                        models.first().map(|m| m.id.clone())
                    }
                };
                let model_id = session.and_then(|s| s.model.clone()).or(default_model);
                model_id.and_then(|id| {
                    models
                        .iter()
                        .find(|m| m.id == id)
                        .and_then(|m| m.context_window)
                        .or_else(|| ModelCatalog::default().context_window(&id))
                })
            };
            match window {
                Some(window) if window > 0 => Some((
                    window,
                    (usage.total_tokens as f64 / window as f64).clamp(0.0, 1.0),
                )),
                _ => None,
            }
        };
        let (context_window_display, context_ratio, estimated) = match context_ratio {
            Some((window, ratio)) => (window, ratio, usage.context_window == 0),
            None => (0, f64::INFINITY, false),
        };
        let ring = if context_ratio.is_finite() {
            // 270° arc gauge drawn as an SVG stroke; gpui renders SVG paths
            // with a fixed viewBox, so the arc is recomputed per ratio.
            let radius = 7.0f64;
            let center = 9.0f64;
            let start_angle = 135.0f64;
            let sweep = 270.0 * context_ratio;
            let point = |angle: f64| {
                let radians = angle.to_radians();
                (
                    center + radius * radians.cos(),
                    center + radius * radians.sin(),
                )
            };
            let (sx, sy) = point(start_angle);
            let (ex, ey) = point(start_angle + sweep);
            let large = if sweep > 180.0 { 1 } else { 0 };
            let color = if context_ratio >= 0.9 {
                palette().danger_text
            } else if context_ratio >= 0.75 {
                palette().warning_text
            } else {
                palette().highlight_text
            };
            let svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 18 18\">\
                 <circle cx=\"9\" cy=\"9\" r=\"7\" fill=\"none\" stroke=\"#{border:06x}\" stroke-width=\"2\"/>\
                 <path d=\"M {sx:.2} {sy:.2} A 7 7 0 {large} 1 {ex:.2} {ey:.2}\" fill=\"none\" stroke=\"#{color:06x}\" stroke-width=\"2\" stroke-linecap=\"round\"/>\
                 </svg>",
                border = palette().border,
                color = color,
            );
            h_flex()
                .id("context-ring")
                .w(px(18.))
                .h(px(18.))
                .flex_shrink_0()
                .child(
                    Icon::default()
                        .data(svg.as_bytes())
                        .size(px(18.))
                        .into_any_element(),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let mut parts: Vec<AnyElement> = vec![];
        if let Some(speed) = tokens_per_second {
            parts.push(
                h_flex()
                    .id("usage-speed")
                    .gap_1()
                    .items_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Zap)
                            .small()
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().muted))
                            .child(format!("{speed} tok/s")),
                    )
                    .into_any_element(),
            );
        }
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
                        .text_size(px(10.))
                        .text_color(rgb(palette().muted))
                        .child(format!(
                            "↑{} ↓{}",
                            format_tokens(usage.input_tokens + usage.cached_input_tokens),
                            format_tokens(usage.output_tokens)
                        )),
                )
                .into_any_element(),
        );
        if context_ratio.is_finite() {
            parts.push(
                h_flex()
                    .id("usage-context")
                    .gap_1()
                    .items_center()
                    .child(ring)
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().muted))
                            .child(format!(
                                "{}{} / {}",
                                if estimated { "≈" } else { "" },
                                format_tokens(usage.total_tokens),
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
                .title("移除工作区")
                .content(move |content, _, _| {
                    let confirm = view.clone();
                    let cancel = view.clone();
                    content.child(
                        v_flex()
                            .gap_3()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .line_height(relative(1.5))
                                    .text_color(rgb(palette().muted))
                                    .child(format!(
                                        "从 Conduit 移除「{name}」？磁盘上的文件与 CLI 自带历史不会被删除；应用内该工作区的会话记录将不再显示。"
                                    )),
                            )
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("remove-workspace-cancel")
                                            .label("取消")
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
                                            .label("移除")
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
            self.message = Some("该工作区仍有任务运行，请先取消后再移除。".into());
            cx.notify();
            return;
        }
        let removed = self.store.workspaces.remove(index);
        self.history_loading.remove(&removed.path);
        self.workspace_drafts.remove(&removed.path);
        self.composer_files
            .retain(|target, _| target.workspace != removed.path);
        self.expanded_workspaces.clear();
        for session in &removed.sessions {
            self.drafts.remove(&session.id);
            self.unread_completed.remove(&session.id);
            self.expanded_errors.retain(|(id, _)| *id != session.id);
            self.expanded_tools.retain(|(id, _)| *id != session.id);
            self.expanded_reasoning.retain(|(id, _)| *id != session.id);
            self.tool_scrolls.retain(|(id, _), _| *id != session.id);
        }
        let removed_ids: Vec<u64> = removed.sessions.iter().map(|session| session.id).collect();
        self.open_tabs.retain(|tab| !removed_ids.contains(tab));
        if self.selected_workspace == Some(index) || self.selected_session.is_none() {
            self.selected_workspace = None;
            self.selected_session = None;
            self.select_fallback_session();
        } else if let Some(selected) = self.selected_workspace
            && selected > index
        {
            self.selected_workspace = Some(selected - 1);
        }
        self.detail_text = None;
        self.persist();
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
                    "取消选择工作区",
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
                    "移除工作区",
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

    /// Whether the tool detail dock is currently visible.
    pub(super) fn details_open(&self) -> bool {
        self.details && self.detail_view == DetailView::Tool
    }

    /// Toggle button for the tool detail dock, pinned to the right of the tab bar.
    pub(super) fn detail_toggle(&self, cx: &mut Context<Self>) -> Button {
        Button::new("toggle-sidebar")
            .ghost()
            .small()
            .compact()
            .icon(gpui_kit::assets::IconName::PanelRight)
            .cursor_pointer()
            .tooltip(if self.details_open() {
                "隐藏工具详情"
            } else {
                "显示工具详情"
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.details = !this.details_open();
                this.detail_view = DetailView::Tool;
                cx.notify();
            }))
    }

    /// Close a session tab. Closing the active tab reveals the next most
    /// recent one; the conversation itself is kept in the sidebar.
    pub(super) fn close_tab(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.selected_session == Some(id) {
            self.drafts
                .insert(id, self.input.read(cx).value().to_string());
            self.remove_tab(id);
            if let Some(next) = self
                .open_tabs
                .last()
                .copied()
                .filter(|next| self.session(*next).is_some())
            {
                let window = cx
                    .windows()
                    .into_iter()
                    .find_map(|handle| handle.downcast::<Self>());
                if let Some(window) = window {
                    let _ = window.update(cx, |_, window, cx| {
                        if let Some(workspace) =
                            self.store.workspaces.iter().position(|workspace| {
                                workspace.sessions.iter().any(|s| s.id == next)
                            })
                        {
                            self.switch(workspace, Some(next), window, cx);
                        }
                    });
                }
            } else {
                self.selected_session = None;
                self.detail_text = None;
                self.message = None;
                cx.notify();
            }
        } else {
            self.remove_tab(id);
            cx.notify();
        }
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
        self.selected_workspace = None;
        self.selected_session = None;
        self.detail_text = None;
        self.message = None;
        self.message_focus = None;
        self.pending_message_jump = false;
        self.timeline_session = None;
        self.input
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        self.sync_git_workspace(cx);
        cx.notify();
    }

    /// Conversation tab strip: one tab per open session, horizontally
    /// scrollable, with a fixed-width action area pinned to the right.
    pub(super) fn session_tab_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let app_entity = cx.entity();
        let active = self.selected_session;
        let mut tabs = h_flex()
            .id("session-tabs")
            .h_full()
            .flex_1()
            .min_w_0()
            .gap(px(4.))
            .pr(px(4.))
            .overflow_x_scroll()
            .lock_scroll_axis();
        for id in &self.open_tabs {
            let id = *id;
            let Some(session) = self.session(id) else {
                continue;
            };
            let selected = active == Some(id);
            let running = self.runs.contains_key(&id);
            let title = if session.title.is_empty() {
                "新会话".to_string()
            } else {
                session.title.clone()
            };
            let title = single_line_title(&title);
            let tab_group = SharedString::from(format!("session-tab-{id}"));
            let tab_app = app_entity.clone();
            tabs = tabs.child(
                h_flex()
                    .id(("session-tab", id))
                    .group(tab_group.clone())
                    .h(px(30.))
                    .my_auto()
                    .px(px(10.))
                    .gap(px(6.))
                    .flex_shrink_0()
                    .items_center()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgb(if selected {
                        palette().border
                    } else {
                        palette().sidebar_bg
                    }))
                    .when(selected, |tab| tab.bg(rgb(palette().surface)))
                    .text_size(px(12.))
                    .text_color(rgb(if selected {
                        palette().text
                    } else {
                        palette().muted
                    }))
                    .cursor_pointer()
                    .hover(|style| style.bg(rgb(palette().surface_hover)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(workspace) = this
                            .store
                            .workspaces
                            .iter()
                            .position(|workspace| workspace.sessions.iter().any(|s| s.id == id))
                        {
                            this.switch(workspace, Some(id), window, cx);
                        }
                    }))
                    .context_menu({
                        move |menu, _, _| {
                            let left_app = tab_app.clone();
                            let right_app = tab_app.clone();
                            let all_app = tab_app.clone();
                            menu.min_w(px(170.))
                                .item(
                                    PopupMenuItem::new("关闭右侧会话")
                                        .icon(IconName::ChevronRight)
                                        .on_click(move |_, _, cx| {
                                            right_app.update(cx, |this, cx| {
                                                this.close_tabs_right(id, cx)
                                            });
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("关闭左侧会话")
                                        .icon(IconName::ChevronLeft)
                                        .on_click(move |_, _, cx| {
                                            left_app.update(cx, |this, cx| {
                                                this.close_tabs_left(id, cx)
                                            });
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new("全部关闭")
                                        .icon(IconName::Delete)
                                        .on_click(move |_, _, cx| {
                                            all_app.update(cx, |this, cx| this.close_all_tabs(cx));
                                        }),
                                )
                        }
                    })
                    .child(provider_icon(session.provider))
                    .child(
                        div()
                            .max_w(px(180.))
                            .truncate()
                            .line_height(px(16.))
                            .whitespace_nowrap()
                            .child(title),
                    )
                    .when(running, |tab| {
                        tab.child(Spinner::new().small().color(rgb(palette().loading).into()))
                    })
                    .child(
                        Button::new(("session-tab-close", id))
                            .ghost()
                            .small()
                            .compact()
                            .icon(gpui_kit::assets::IconName::X)
                            .opacity(0.)
                            .group_hover(tab_group, |style| style.opacity(1.))
                            .focus_visible(|style| style.opacity(1.))
                            .cursor_pointer()
                            .tooltip(if running {
                                "停止并关闭会话"
                            } else {
                                "关闭会话"
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_tab(id, cx);
                                cx.stop_propagation();
                            })),
                    ),
            );
        }
        if self.open_tabs.is_empty() {
            tabs = tabs.child(
                h_flex()
                    .h_full()
                    .px_3()
                    .items_center()
                    .text_size(px(12.))
                    .text_color(rgb(palette().faint))
                    .child("未选择工作区"),
            );
        }
        h_flex()
            .h(px(44.))
            .pl_2()
            .pr_3()
            .border_b_1()
            .border_color(rgb(palette().border))
            .items_center()
            .child(tabs)
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(div().w(px(1.)).h(px(18.)).mx_1().bg(rgb(palette().border)))
                    .child(self.detail_toggle(cx)),
            )
            .into_any_element()
    }

    pub(super) fn switch(
        &mut self,
        workspace: usize,
        session: Option<u64>,
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
        self.selected_workspace = Some(workspace);
        self.selected_session = session;
        if let Some(id) = session {
            self.unread_completed.remove(&id);
            self.push_tab(id);
        }
        self.message_focus = None;
        self.pending_message_jump = false;
        self.detail_text = None;
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
        if session.is_some() {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
        self.timeline_session = None;
        // A newly selected conversation opens at its latest message.
        self.at_bottom = true;
        if let Some(provider) = session
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
            prompt: Some("选择你信任的工作区目录".into()),
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
                            _ => this.message = Some("目录不存在或不可访问".into()),
                        }
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            other => {
                let _ = this.update(cx, |this, cx| {
                    this.message = Some(format!("打开目录失败：{other:?}"));
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
        cx.spawn_in(window, async move |this, cx| {
            let scan_path = path.clone();
            let imported = cx
                .background_executor()
                .spawn(async move { crate::history::scan_workspace(&scan_path) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_history_import(&path, imported, true, window, cx);
            });
        })
        .detach();
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
        for path in &paths {
            self.history_loading.insert(path.clone());
        }
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let scan_paths = paths.clone();
            let imported = cx
                .background_executor()
                .spawn(async move { crate::history::scan_workspaces(&scan_paths) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                for (path, sessions) in paths.into_iter().zip(imported) {
                    this.finish_history_import(&path, sessions, false, window, cx);
                }
            });
        })
        .detach();
    }

    pub(super) fn finish_history_import(
        &mut self,
        path: &Path,
        imported: Vec<crate::history::ImportedSession>,
        persist: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.history_loading.remove(path);
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
        let imported = imported
            .into_iter()
            .filter(|imported| {
                !self.store.workspaces[index].sessions.iter().any(|session| {
                    session.provider == imported.provider
                        && session.native_id.as_deref() == Some(imported.native_id.as_str())
                        && self.runs.contains_key(&session.id)
                })
            })
            .collect::<Vec<_>>();
        let discovered = !imported.is_empty();
        crate::history::merge_imported(
            &mut self.store.workspaces[index],
            &mut self.next_id,
            imported,
        );
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
        let id = self.next_id;
        self.next_id += 1;
        self.catalogs.insert(provider, models::load(provider));
        self.ccs = ccs::load();
        self.store.sync_ccs_providers(&self.ccs);
        let model = self
            .catalogs
            .get(&provider)
            .and_then(|catalog| catalog.current.clone());
        self.store.workspaces[index].sessions.insert(
            0,
            SessionRecord {
                id,
                provider,
                created_at: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                exit_code: None,
                title: "新会话".into(),
                archived: false,
                native_id: None,
                model,
                messages: vec![],
                interrupted: false,
                channel: Default::default(),
                reasoning_effort: Default::default(),
                permission: PermissionLevel::default(),
                usage: Usage::default(),
            },
        );
        self.selected_workspace = Some(index);
        self.selected_session = Some(id);
        self.push_tab(id);
        self.new_session_provider = provider;
        self.remember_provider(provider);
        self.persist();
        cx.notify();
        Some(id)
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
    pub(super) fn session_heatmap(&self, weeks: usize) -> AnyElement {
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
                let label: SharedString =
                    format!("{} · {} 次会话", day.format("%Y-%m-%d"), count).into();
                column = column.child(
                    div()
                        .id(("heatmap-cell", index))
                        .w(px(12.))
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
                    .text_size(px(9.))
                    .text_color(rgb(palette().faint))
                    .child(weekday),
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
                    .text_size(px(30.))
                    .font_semibold()
                    .text_color(rgb(palette().text))
                    .child("连接一个本地工作区"),
            )
            .child(
                v_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette().muted))
                            .child(format!("近 {weeks} 周共 {total} 次会话")),
                    )
                    .child(h_flex().gap_2().child(weekday_labels).child(grid)),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().faint))
                    .child("CLI 只会在所选目录内执行任务"),
            )
            .into_any_element()
    }
}
