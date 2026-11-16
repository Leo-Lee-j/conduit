//! A visible, independently collapsible child-agent panel above the composer.
use super::*;
use crate::subagents::Status;

impl ConduitApp {
    pub(super) fn subagent_panel(
        &mut self,
        session_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        if session.subagents.is_empty() {
            return None;
        }
        let run = self
            .session_control
            .runs
            .get(&session_id)
            .map(|run| run.prefix.as_str())
            .filter(|run| session.subagents.iter().any(|agent| agent.run == *run))
            .unwrap_or_else(|| session.subagents.last().unwrap().run.as_str());
        let history_key = (session_id, "history".to_string());
        let history = self.session_control.expanded_agents.contains(&history_key);
        let has_history = session.subagents.iter().any(|agent| agent.run != run);
        let mut agents: Vec<_> = session
            .subagents
            .iter()
            .filter(|agent| history || agent.run == run)
            .cloned()
            .collect();
        agents.sort_by_key(|agent| (!agent.status.active(), agent.started_at));
        let active = agents.iter().filter(|agent| agent.status.active()).count();
        let completed = agents
            .iter()
            .filter(|agent| agent.status == Status::Completed)
            .count();
        let failed = agents
            .iter()
            .filter(|agent| agent.status == Status::Failed)
            .count();
        let collapsed = self.session_control.collapsed_agents.contains(&session_id);
        let title = conduit::i18n::format(
            "subagents.heading",
            &[
                ("total", agents.len().to_string()),
                ("active", active.to_string()),
                ("completed", completed.to_string()),
                ("failed", failed.to_string()),
            ],
        );
        let header = Button::new(SharedString::from(format!("subagents-toggle-{session_id}")))
            .ghost()
            .compact()
            .icon(if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .child(title)
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.session_control.collapsed_agents.remove(&session_id) {
                    this.session_control.collapsed_agents.insert(session_id);
                }
                cx.notify();
            }));
        let mut heading = h_flex().w_full().justify_between().child(header);
        if has_history {
            heading = heading.child(
                Button::new("subagents-history")
                    .ghost()
                    .compact()
                    .child(conduit::i18n::text_static(if history {
                        "subagents.current"
                    } else {
                        "subagents.history"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.session_control.expanded_agents.remove(&history_key) {
                            this.session_control
                                .expanded_agents
                                .insert(history_key.clone());
                        }
                        cx.notify();
                    })),
            );
        }
        let mut panel = v_flex()
            .debug_selector(|| "subagent-panel".into())
            .w_full()
            .gap_1()
            .child(heading);
        if !collapsed {
            let mut rows = v_flex()
                .id("subagent-rows")
                .debug_selector(|| "subagent-rows".into())
                .w_full()
                .max_h(px(208.))
                .overflow_y_scroll()
                .gap_1();
            for agent in agents {
                let identity = format!("{}:{}", agent.run, agent.id);
                let expanded = self
                    .session_control
                    .expanded_agents
                    .contains(&(session_id, identity.clone()));
                let label = if !agent.task.is_empty() {
                    agent.task.lines().next().unwrap_or_default()
                } else if !agent.name.is_empty() {
                    &agent.name
                } else {
                    &agent.id
                };
                let label: String = label.chars().take(120).collect();
                let duration = agent.elapsed_ms.map(Duration::from_millis).or_else(|| {
                    (agent.started_at > 0).then(|| {
                        Duration::from_secs(
                            agent
                                .finished_at
                                .unwrap_or_else(|| {
                                    if agent.status.active() {
                                        unix_now()
                                    } else {
                                        agent.updated_at
                                    }
                                })
                                .saturating_sub(agent.started_at),
                        )
                    })
                });
                let color = match agent.status {
                    Status::Failed => palette().diff_remove_text,
                    Status::Completed => palette().diff_add_text,
                    Status::WaitingApproval | Status::WaitingInput => palette().text,
                    _ => palette().muted,
                };
                let mut content = h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .min_w(px(72.))
                            .text_color(rgb(color))
                            .child(agent.status.label()),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(label));
                if !agent.activity.is_empty() {
                    content = content.child(
                        div()
                            .max_w(px(190.))
                            .truncate()
                            .text_color(rgb(palette().muted))
                            .child(agent.activity.clone()),
                    );
                }
                if let Some(duration) = duration {
                    content = content.child(
                        div()
                            .text_color(rgb(palette().faint))
                            .child(format!("{}s", duration.as_secs())),
                    );
                }
                let button = Button::new(SharedString::from(format!("subagent-{identity}")))
                    .ghost()
                    .compact()
                    .w_full()
                    .child(content)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let key = (session_id, identity.clone());
                        if !this.session_control.expanded_agents.remove(&key) {
                            this.session_control.expanded_agents.insert(key);
                        }
                        cx.notify();
                    }));
                let mut row = v_flex().w_full().child(button);
                if expanded {
                    row = row.child(
                        v_flex()
                            .px_3()
                            .py_2()
                            .gap_1()
                            .text_color(rgb(palette().muted))
                            .child(format!("{} · {}", agent.name, agent.id))
                            .child(agent.task)
                            .child(agent.result),
                    );
                }
                rows = rows.child(row);
            }
            panel = panel.child(rows);
        }
        Some(
            div()
                .w_full()
                .px_6()
                .pb_2()
                .child(panel.when(!self.store.ui.full_width_content, |p| {
                    p.max_w(px(840.)).mx_auto()
                }))
                .into_any_element(),
        )
    }
}
