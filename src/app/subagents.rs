//! Event-backed child tasks, sharing the composer's outer frame.
use super::*;
use crate::subagents::{Agent, Status};
pub(super) mod navigation;

#[derive(Default)]
pub(super) struct PanelState {
    scope: String,
    all_completed: bool,
    expanded: bool,
    reveal: bool,
    selection_focus: Option<FocusHandle>,
}

impl PanelState {
    fn sync(&mut self, scope: String, all_completed: bool) {
        let new_scope = self.scope != scope;
        if new_scope || self.all_completed != all_completed {
            self.expanded = new_scope || !all_completed;
            self.scope = scope;
            self.all_completed = all_completed;
        }
        if std::mem::take(&mut self.reveal) {
            self.expanded = true;
        }
    }
}
fn priority(status: Status) -> u8 {
    match status {
        Status::Failed => 0,
        Status::Stopped => 1,
        Status::WaitingApproval | Status::WaitingInput => 2,
        Status::Starting | Status::Running => 3,
        Status::Idle | Status::Unknown => 4,
        Status::Completed => 5,
    }
}
fn status_color(status: Status) -> u32 {
    match status {
        Status::Failed => palette().danger_text,
        Status::Stopped | Status::WaitingApproval | Status::WaitingInput => palette().warning_text,
        Status::Starting | Status::Running => palette().activity_running,
        _ => palette().muted,
    }
}
fn status_icon(status: Status) -> AnyElement {
    let color = rgb(status_color(status));
    if matches!(status, Status::Starting | Status::Running) {
        return Spinner::new()
            .small()
            .color(color.into())
            .into_any_element();
    }
    Icon::new(match status {
        Status::Completed => FullIconName::CircleCheck,
        Status::Failed => FullIconName::CircleX,
        Status::Stopped => FullIconName::CircleStop,
        Status::WaitingApproval | Status::WaitingInput => FullIconName::CirclePause,
        _ => FullIconName::CircleDashed,
    })
    .size(px(15.))
    .text_color(color)
    .into_any_element()
}
fn title(agent: &Agent) -> (&str, bool) {
    if let Some(title) = agent
        .title
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
    {
        return (title, false);
    }
    let fallback = agent
        .task
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .or_else(|| (!agent.name.trim().is_empty()).then_some(agent.name.trim()))
        .unwrap_or(&agent.id);
    (fallback, true)
}
fn duration(agent: &Agent, now: u64) -> Option<String> {
    if agent.status == Status::Stopped {
        return None;
    }
    let elapsed = if agent.status.active() && agent.started_at > 0 {
        // A live clock advances even while no new progress frame arrives.
        Duration::from_secs(now.saturating_sub(agent.started_at))
    } else if let Some(elapsed) = agent.elapsed_ms {
        Duration::from_millis(elapsed)
    } else if agent.started_at > 0 {
        Duration::from_secs(
            agent
                .finished_at
                .unwrap_or(agent.updated_at)
                .saturating_sub(agent.started_at),
        )
    } else {
        return None;
    };
    Some(super::chat::format_elapsed(elapsed, false))
}

fn disclosure_label(label: &str, expanded: bool) -> String {
    let action = conduit::i18n::text_static(if expanded {
        "chat.collapse"
    } else {
        "chat.expand"
    });
    format!("{action} · {label}")
}

impl ConduitApp {
    pub(super) fn subagent_panel(
        &mut self,
        session_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        let latest = session.subagents.last()?;
        let run = self
            .session_control
            .runs
            .get(&session_id)
            .map(|run| run.prefix.as_str())
            .filter(|run| session.subagents.iter().any(|agent| agent.run == *run))
            .unwrap_or(&latest.run)
            .to_owned();
        let history_key = (session_id, "history".to_string());
        let history = self.session_control.expanded_agents.contains(&history_key);
        let has_history = session.subagents.iter().any(|agent| agent.run != run);
        let mut agents: Vec<_> = session
            .subagents
            .iter()
            .filter(|agent| history || agent.run == run)
            .cloned()
            .collect();
        agents.sort_by_key(|agent| (priority(agent.status), agent.started_at));
        let all_completed = agents.iter().all(|agent| agent.status == Status::Completed);
        let mut counts: Vec<(Status, usize)> = Vec::new();
        for agent in &agents {
            if let Some((_, count)) = counts
                .iter_mut()
                .find(|(status, _)| *status == agent.status)
            {
                *count += 1;
            } else {
                counts.push((agent.status, 1));
            }
        }
        let summary = counts
            .iter()
            .map(|(status, count)| format!("{count} {}", status.label()))
            .collect::<Vec<_>>()
            .join(" · ");
        let state = self
            .session_control
            .agent_panels
            .entry(session_id)
            .or_default();
        state.sync(format!("{run}:history={history}"), all_completed);
        let expanded = state.expanded;
        let selection_focus = state
            .selection_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let heading = conduit::i18n::format(
            if all_completed {
                "subagents.all_completed"
            } else {
                "subagents.title"
            },
            &[("total", agents.len().to_string())],
        );
        let toggle = Button::new(SharedString::from(format!("subagents-toggle-{session_id}")))
            .ghost()
            .compact()
            .h(px(36.))
            .min_w_0()
            .accessibility_label(disclosure_label(
                &format!("{heading} · {summary}"),
                expanded,
            ))
            .icon(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .when(all_completed, |row| {
                        row.child(
                            Icon::new(FullIconName::CircleCheck)
                                .size(px(15.))
                                .text_color(rgb(palette().diff_add_text)),
                        )
                    })
                    .child(div().truncate().child(heading)),
            )
            .tooltip(summary.clone())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                if let Some(state) = this.session_control.agent_panels.get_mut(&session_id) {
                    state.expanded = !state.expanded;
                }
                cx.notify();
            }));
        let mut header = h_flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_2()
            .px_2()
            .child(toggle);
        if !all_completed {
            // Keep chrome bounded for large batches; the summary retains exact counts.
            let chunk = agents.len().div_ceil(12).max(1);
            let segments = agents.chunks(chunk).map(|group| {
                div()
                    .w(px(12.))
                    .h(px(4.))
                    .rounded_full()
                    .bg(rgb(status_color(group[0].status)))
            });
            header = header.child(
                h_flex()
                    .max_w(px(94.))
                    .min_w_0()
                    .overflow_hidden()
                    .gap(px(3.))
                    .children(segments),
            );
        }
        header = header.child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(ui_text_size(12.))
                .text_color(rgb(palette().muted))
                .when(!all_completed, |row| row.child(summary)),
        );
        if has_history {
            header = header.child(
                Button::new("subagents-history")
                    .ghost()
                    .compact()
                    .h(px(32.))
                    .label(conduit::i18n::text_static(if history {
                        "subagents.current"
                    } else {
                        "subagents.history"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !this.session_control.expanded_agents.remove(&history_key) {
                            this.session_control
                                .expanded_agents
                                .insert(history_key.clone());
                        }
                        if let Some(state) = this.session_control.agent_panels.get_mut(&session_id)
                        {
                            state.reveal = true;
                        }
                        cx.notify();
                    })),
            );
        }
        let mut panel = v_flex()
            .id("subagent-panel")
            .debug_selector(|| "subagent-panel".into())
            .track_focus(&selection_focus)
            .w_full()
            .min_w_0()
            .rounded_t(px(10.))
            .bg(rgb(palette().surface_strong))
            .on_click(|_, _, cx| cx.stop_propagation())
            .border_b_1()
            .border_color(rgb(palette().border))
            .child(header);
        if expanded {
            let typography = self.chat_typography();
            let now = unix_now();
            let mut rows = v_flex()
                .id("subagent-rows")
                .debug_selector(|| "subagent-rows".into())
                .w_full()
                .max_h(px(208.))
                .overflow_y_scroll();
            for agent in agents {
                let identity = format!("{}:{}", agent.run, agent.id);
                let key = (session_id, identity.clone());
                let detail_expanded = self.session_control.expanded_agents.contains(&key);
                let (label, fallback) = title(&agent);
                let mut content = h_flex().w_full().min_w_0().items_center().gap_2().child(
                    div()
                        .w(px(16.))
                        .flex_shrink_0()
                        .child(status_icon(agent.status)),
                );
                if !agent.role.trim().is_empty() {
                    content = content.child(
                        div()
                            .max_w(px(88.))
                            .truncate()
                            .rounded(px(5.))
                            .px_2()
                            .py(px(1.))
                            .bg(rgb(palette().surface_hover))
                            .text_color(rgb(palette().muted))
                            .text_size(ui_text_size(12.))
                            .child(agent.role.clone()),
                    );
                }
                content = content.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(rgb(if fallback {
                            palette().muted
                        } else {
                            palette().text
                        }))
                        .child(label.to_owned()),
                );
                if let Some(elapsed) = duration(&agent, now) {
                    content = content.child(
                        div()
                            .flex_shrink_0()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(status_color(agent.status)))
                            .child(elapsed),
                    );
                }
                if agent.status != Status::Completed
                    && !matches!(agent.status, Status::Running | Status::Starting)
                {
                    content = content.child(
                        div()
                            .flex_shrink_0()
                            .text_size(ui_text_size(12.))
                            .text_color(rgb(status_color(agent.status)))
                            .child(agent.status.label()),
                    );
                }
                let navigation_run = agent.run.clone();
                let navigation_id = agent.id.clone();
                let open = Button::new(SharedString::from(format!("subagent-open-{identity}")))
                    .ghost()
                    .compact()
                    .flex_1()
                    .min_w_0()
                    .h(px(40.))
                    .px_3()
                    .accessibility_label(conduit::i18n::format(
                        "subagents.open",
                        &[("title", label.to_owned())],
                    ))
                    .child(content)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.open_subagent(
                            session_id,
                            navigation_run.clone(),
                            navigation_id.clone(),
                            window,
                            cx,
                        );
                    }));
                let content = div().child(
                    Icon::new(if detail_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size(px(12.))
                    .text_color(rgb(palette().muted)),
                );
                let button = Button::new(SharedString::from(format!("subagent-{identity}")))
                    .ghost()
                    .compact()
                    .w(px(36.))
                    .h(px(40.))
                    .accessibility_label(disclosure_label(
                        &if agent.role.trim().is_empty() {
                            format!("{}: {label}", agent.status.label())
                        } else {
                            format!("{} · {}: {label}", agent.role.trim(), agent.status.label())
                        },
                        detail_expanded,
                    ))
                    .tooltip(format!("{}: {label}", agent.status.label()))
                    .child(content)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !this.session_control.expanded_agents.remove(&key) {
                            this.session_control.expanded_agents.insert(key.clone());
                        }
                        cx.notify();
                    }));
                let mut row = v_flex()
                    .w_full()
                    .min_w_0()
                    .border_t_1()
                    .border_color(rgb(palette().border))
                    .child(h_flex().w_full().min_w_0().child(open).child(button));
                if detail_expanded {
                    let mut detail = v_flex()
                        .w_full()
                        .min_w_0()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .bg(rgb(palette().surface))
                        .text_size(ui_text_size(13.))
                        .text_color(rgb(palette().muted))
                        .on_mouse_move(cx.listener(|_, event: &MouseMoveEvent, _, cx| {
                            if event.pressed_button == Some(MouseButton::Left) {
                                cx.notify();
                            }
                        }))
                        .on_mouse_up(MouseButton::Left, {
                            let focus = selection_focus.clone();
                            move |_, window, cx| {
                                // Let the root Copy action handle the selection instead
                                // of the composer's input, which can still hold focus.
                                if !TextSelection::selected_text(window, cx).is_empty() {
                                    focus.focus(window, cx);
                                }
                            }
                        });
                    if !agent.task.trim().is_empty() {
                        detail = detail
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(conduit::i18n::text_static("subagents.task"))
                                    .child(
                                        Clipboard::new(SharedString::from(format!(
                                            "copy-task-{identity}"
                                        )))
                                        .tooltip(conduit::i18n::text_static("subagents.copy_task"))
                                        .accessibility_label(conduit::i18n::text_static(
                                            "subagents.copy_task",
                                        ))
                                        .value(agent.task.clone()),
                                    ),
                            )
                            .child(gpui_kit::base::SelectableText::new(
                                SharedString::from(format!("task-{identity}")),
                                agent.task.clone(),
                            ));
                    }
                    if !agent.activity.trim().is_empty() {
                        detail = detail.child(gpui_kit::base::SelectableText::new(
                            SharedString::from(format!("activity-{identity}")),
                            agent.activity.clone(),
                        ));
                    }
                    if !agent.result.trim().is_empty() {
                        let owner = cx.weak_entity();
                        detail = detail
                            .child(conduit::i18n::text_static("subagents.output"))
                            .child(
                                chat_markdown_view(
                                    format!("subagent-result-{identity}").into(),
                                    agent.result.clone(),
                                    &typography,
                                    &format!("subagent-code-{identity}"),
                                    true,
                                )
                                .on_link_click(
                                    move |url, event, window, cx| {
                                        if !super::chat::activates_link(event) {
                                            return;
                                        }
                                        let _ = owner.update(cx, |this, cx| {
                                            this.open_chat_link(session_id, url, window, cx);
                                        });
                                    },
                                ),
                            );
                    }
                    row = row.child(detail);
                }
                rows = rows.child(row);
            }
            panel = panel.child(rows);
        }
        Some(panel.into_any_element())
    }
}

#[cfg(all(test, feature = "ui-tests"))]
#[path = "subagents/tests.rs"]
mod ui_tests;

#[cfg(test)]
mod tests {
    use super::PanelState;

    #[test]
    fn completed_history_scope_starts_expanded() {
        let mut state = PanelState::default();
        let scope: String = ['h', 'i', 's', 't', 'o', 'r', 'y'].into_iter().collect();
        state.sync(scope, true);
        assert!(state.expanded);
    }

    // Completion may shrink the panel once, but must not undo an explicit
    // request to inspect results, or carry a collapsed batch into a new run.
    #[test]
    fn completion_and_batch_changes_preserve_explicit_inspection() {
        let mut state = PanelState::default();
        state.sync("run-a".into(), false);
        assert!(state.expanded);
        state.sync("run-a".into(), true);
        assert!(!state.expanded);
        state.expanded = true;
        state.sync("run-a".into(), true);
        assert!(state.expanded);
        state.expanded = false;
        state.sync("run-b".into(), false);
        assert!(state.expanded);
        state.reveal = true;
        state.sync("history".into(), true);
        assert!(state.expanded);
    }
}
