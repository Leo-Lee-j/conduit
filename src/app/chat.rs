//! Transcript rows, message actions, and turn statistics.

use super::*;
use std::rc::Rc;

#[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
pub(in crate::app) mod profiling;
pub(super) mod rows;
mod turn_diff;

/// Text segments separated by tools still belong to one user-facing reply.
/// Derive the bounds from user messages so imported histories work as well.
pub(super) fn assistant_reply_range(
    session: &SessionRecord,
    index: usize,
) -> std::ops::Range<usize> {
    let start = session.messages[..index]
        .iter()
        .rposition(|item| item.kind == ItemKind::User)
        .map_or(0, |position| position + 1);
    let end = session.messages[index..]
        .iter()
        .position(|item| item.kind == ItemKind::User)
        .map_or(session.messages.len(), |position| index + position);
    start..end
}

pub(super) fn assistant_reply_text(session: &SessionRecord, index: usize) -> String {
    session.messages[assistant_reply_range(session, index)]
        .iter()
        .filter(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty())
        .map(|item| item.text.as_str())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Keep short counters in seconds and longer turns readable as minutes/seconds.
fn format_elapsed(elapsed: Duration, precise: bool) -> String {
    if elapsed > Duration::from_secs(60) {
        let seconds = elapsed.as_secs();
        conduit::i18n::format(
            "chat.m_s",
            &[
                ("arg0", format!("{}", seconds / 60)),
                ("arg1", format!("{:02}", seconds % 60)),
            ],
        )
    } else if precise {
        format!("{:.1} s", elapsed.as_secs_f64())
    } else {
        format!("{} s", elapsed.as_secs())
    }
}

/// Old histories have only message timestamps, which measure time to the last
/// visible reply rather than CLI completion. Keep that fallback visibly approximate.
pub(super) fn reply_elapsed_label(
    session: &SessionRecord,
    index: usize,
    live: bool,
) -> Option<String> {
    let range = assistant_reply_range(session, index);
    if let Some(elapsed) = session.messages[range.clone()]
        .iter()
        .filter(|item| item.kind == ItemKind::Assistant)
        .find_map(|item| item.elapsed_ms)
    {
        return Some(conduit::i18n::format(
            "chat.duration",
            &[(
                "arg0",
                format_elapsed(Duration::from_millis(elapsed), true).to_string(),
            )],
        ));
    }
    if live && range.end == session.messages.len() {
        return None;
    }
    let start = range
        .start
        .checked_sub(1)
        .and_then(|position| session.messages.get(position))?;
    let end = session.messages[range]
        .iter()
        .rev()
        .find(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty())?;
    if start.created_at == 0 || end.created_at == 0 {
        return None;
    }
    let elapsed = end.created_at.checked_sub(start.created_at)?;
    Some(conduit::i18n::format(
        "chat.approx_duration",
        &[(
            "arg0",
            format_elapsed(Duration::from_secs(elapsed), false).to_string(),
        )],
    ))
}

impl ConduitApp {
    /// Shows the copied check mark on `id` for a short moment. Each call gets a
    /// fresh generation so a previous timer cannot hide newer feedback.
    pub(super) fn show_copied(
        &mut self,
        id: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copied_action_seq = self.copied_action_seq.wrapping_add(1);
        let seq = self.copied_action_seq;
        self.copied_action = Some(id);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1500))
                .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                if this.copied_action_seq == seq {
                    this.copied_action = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Live engine activity above the composer. Turn steps and file churn live
    /// in [`Self::turn_stats_tags`], pinned to the composer's top-right corner.
    pub(super) fn turn_activity_strip(&self, session_id: u64) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        let run = self.session_control.runs.get(&session_id)?;
        let model = run
            .model
            .as_deref()
            .map(|model| {
                self.catalogs
                    .get(&session.provider)
                    .map_or_else(|| model.to_string(), |catalog| catalog.label(model))
            })
            .unwrap_or_else(|| conduit::i18n::text_static("chat.cli_default_model").into());
        let effort = if matches!(session.provider, Provider::OpenCode | Provider::Qoder) {
            run.acp_variant
                .clone()
                .unwrap_or_else(|| conduit::i18n::text_static("chat.default").into())
        } else {
            run.reasoning_effort.label().to_owned()
        };
        Some(
            h_flex()
                .id("engine-activity")
                .debug_selector(|| "engine-activity".into())
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(provider_icon(session.provider))
                .child(
                    div()
                        .id("engine-activity-text")
                        .debug_selector(|| "engine-activity-text".into())
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(palette().muted))
                        .child(run.activity_label()),
                )
                .child(
                    div()
                        .id("engine-activity-spinner")
                        .debug_selector(|| "engine-activity-spinner".into())
                        .child(Spinner::new().small().color(rgb(palette().loading).into())),
                )
                .child(
                    div()
                        .debug_selector(|| "engine-elapsed".into())
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child(format_elapsed(run.turn_started.elapsed(), false)),
                )
                .child(
                    div()
                        .debug_selector(|| "engine-model".into())
                        .min_w_0()
                        .max_w(px(240.))
                        .truncate()
                        .text_size(px(11.))
                        .text_color(rgb(palette().muted))
                        .child(model),
                )
                .child(
                    div()
                        .debug_selector(|| "engine-reasoning-effort".into())
                        .flex_shrink_0()
                        .text_size(px(11.))
                        .text_color(rgb(palette().faint))
                        .child(conduit::i18n::format(
                            "chat.reasoning_effort",
                            &[("effort", effort.to_string())],
                        )),
                )
                .into_any_element(),
        )
    }

    /// Step and file-churn tags for the latest turn, docked above the
    /// composer's top-right corner. Clicking them opens the per-step and
    /// per-file details above the tags; clicking outside closes the panel.
    pub(super) fn turn_stats_tags(
        &mut self,
        session_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        let start = session
            .messages
            .iter()
            .rposition(|m| m.kind == ItemKind::User)
            .unwrap_or(0);
        let items: Vec<_> = session.messages[start..]
            .iter()
            .filter(|m| m.kind == ItemKind::Tool)
            .cloned()
            .collect();
        let Some((stats, historical_plan)) = self.tool_analysis.turn(session_id, &items, cx) else {
            return Some(
                div()
                    .text_size(px(11.))
                    .text_color(rgb(palette().muted))
                    .child(conduit::i18n::text_static("chat.counting_changes"))
                    .into_any_element(),
            );
        };
        let session = self.session(session_id)?;
        let plan = self
            .session_control
            .plans
            .get(&session_id)
            .cloned()
            .filter(|steps| !steps.is_empty())
            .unwrap_or(historical_plan);
        if plan.is_empty() && stats.files.is_empty() {
            return None;
        }
        let live = self.session_control.runs.contains_key(&session_id);
        let completed = plan
            .iter()
            .filter(|step| step.status == PlanStatus::Completed)
            .count();
        let total = plan.len();
        let root = self
            .store
            .workspaces
            .iter()
            .find(|workspace| workspace.sessions.iter().any(|item| item.id == session_id))
            .map(|workspace| workspace.path.to_string_lossy().into_owned());
        let mut tags = h_flex()
            .id("turn-stats")
            .debug_selector(|| "turn-stats".into())
            .items_center()
            .gap(px(6.));
        if total > 0 {
            let done = completed == total;
            tags =
                tags.child(
                    stat_tag()
                        .child(
                            Icon::new(if done {
                                FullIconName::CircleCheck
                            } else {
                                FullIconName::CircleDashed
                            })
                            .size(px(11.))
                            .text_color(rgb(if done {
                                palette().diff_add_text
                            } else {
                                palette().faint
                            })),
                        )
                        .child(div().text_color(rgb(palette().muted)).child(
                            conduit::i18n::format(
                                "chat.step",
                                &[
                                    ("completed", format!("{}", completed)),
                                    ("total", format!("{}", total)),
                                ],
                            ),
                        )),
                );
        }
        if !stats.files.is_empty() {
            tags = tags.child(
                stat_tag()
                    .child(
                        Icon::new(IconName::File)
                            .size(px(11.))
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().muted))
                            .child(conduit::i18n::format(
                                "chat.files",
                                &[("arg0", format!("{}", stats.files.len()))],
                            )),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().diff_add_text))
                            .child(format!("+{}", stats.added)),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette().diff_remove_text))
                            .child(format!("-{}", stats.removed)),
                    ),
            );
        }
        let trigger = Button::new(SharedString::from(format!(
            "turn-stats-trigger-{session_id}"
        )))
        .ghost()
        .h(px(24.))
        .px(px(0.))
        .rounded(px(8.))
        .cursor_pointer()
        .accessibility_label(conduit::i18n::text_static(
            "chat.view_steps_and_file_changes",
        ))
        .child(tags);
        let panel_plan = plan;
        let panel_stats = stats;
        let app = cx.weak_entity();
        let turn_key = session
            .messages
            .iter()
            .rfind(|item| item.kind == ItemKind::User)
            .map(|item| item.key.clone());
        Some(
            Popover::new(SharedString::from(format!("turn-stats-{session_id}")))
                .anchor(Anchor::BottomRight)
                .offset(px(8.))
                .appearance(false)
                .trigger(trigger)
                .content(move |_, window, cx| {
                    let app = app.clone();
                    let turn_key = turn_key.clone();
                    let popover = cx.entity().downgrade();
                    turn_stats_details(
                        &panel_plan,
                        &panel_stats,
                        root.as_deref(),
                        live,
                        window,
                        move |path, window, cx| {
                            let _ = popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                            let _ = app.update(cx, |app, cx| {
                                app.open_turn_diff(
                                    session_id,
                                    turn_key.as_deref(),
                                    path,
                                    window,
                                    cx,
                                );
                            });
                        },
                    )
                })
                .into_any_element(),
        )
    }
}

fn stat_tag() -> Div {
    h_flex()
        .h(px(24.))
        .px(px(8.))
        .gap(px(5.))
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface_strong))
        .text_size(px(11.))
        .line_height(px(15.))
}

/// Steps and per-file churn of the latest turn, shown in a popover above the
/// stats tags above the composer.
fn turn_stats_details(
    plan: &[PlanStep],
    stats: &TurnFileStats,
    root: Option<&str>,
    live: bool,
    window: &Window,
    open_diff: impl Fn(&str, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let open_diff = Rc::new(open_diff);
    let mut rows = v_flex()
        .id("turn-stats-details")
        .debug_selector(|| "turn-stats-details".into())
        .w(px(
            (f32::from(window.viewport_size().width) - 32.).clamp(200., 380.)
        ))
        .max_h(px(
            (f32::from(window.viewport_size().height) - 100.).clamp(120., 320.)
        ))
        .overflow_y_scroll()
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .p_3()
        .gap_1()
        .rounded(px(12.))
        .border_1()
        .border_color(rgb(palette().border))
        .bg(rgb(palette().surface))
        .shadow_sm();
    for step in plan {
        let icon = match step.status {
            PlanStatus::Completed => Icon::new(IconName::CircleCheck)
                .size(px(13.))
                .text_color(rgb(palette().diff_add_text))
                .into_any_element(),
            PlanStatus::InProgress if live => Spinner::new()
                .small()
                .color(rgb(palette().muted).into())
                .into_any_element(),
            PlanStatus::InProgress => Icon::new(FullIconName::CircleDot)
                .size(px(13.))
                .text_color(rgb(palette().warning_text))
                .into_any_element(),
            PlanStatus::Pending => Icon::new(FullIconName::CircleDashed)
                .size(px(13.))
                .text_color(rgb(palette().faint))
                .into_any_element(),
        };
        rows = rows.child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(icon)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(rgb(match step.status {
                            PlanStatus::Completed => palette().faint,
                            PlanStatus::InProgress => palette().text,
                            PlanStatus::Pending => palette().muted,
                        }))
                        .child(step.text.clone()),
                ),
        );
    }
    if !plan.is_empty() && !stats.files.is_empty() {
        rows = rows.child(div().w_full().h(px(1.)).my_1().bg(rgb(palette().border)));
    }
    if !stats.files.is_empty() {
        rows = rows.child(
            h_flex()
                .px_2()
                .pb_1()
                .items_center()
                .justify_between()
                .text_size(px(11.))
                .text_color(rgb(palette().faint))
                .child(conduit::i18n::format(
                    "chat.file_changes_in_this_turn",
                    &[("arg0", format!("{}", stats.files.len()))],
                ))
                .child(conduit::i18n::text_static("chat.click_to_view_diff")),
        );
    }
    for (index, (path, stat)) in stats.files.iter().enumerate() {
        let path = path.clone();
        let display_path = workspace_relative_path(&path, root);
        let open_diff = open_diff.clone();
        rows = rows.child(
            Button::new(("turn-stats-file", index))
                .debug_selector(move || format!("turn-stats-file-{index}"))
                .ghost()
                .w_full()
                .min_w_0()
                .h(px(34.))
                .flex_shrink_0()
                .px_2()
                .rounded(px(6.))
                .cursor_pointer()
                .tooltip(conduit::i18n::format(
                    "chat.view_changes_in_this_turn",
                    &[("display_path", display_path.to_string())],
                ))
                .accessibility_label(conduit::i18n::format(
                    "chat.view_this_turn_s_diff_for",
                    &[("display_path", display_path.to_string())],
                ))
                .on_click(move |_, window, cx| open_diff(&path, window, cx))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(FullIconName::FileDiff)
                                .size(px(14.))
                                .text_color(rgb(palette().faint)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_left()
                                .text_size(px(12.))
                                .text_color(rgb(palette().text))
                                .child(display_path),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(palette().diff_add_text))
                                .child(format!("+{}", stat.added)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(palette().diff_remove_text))
                                .child(format!("-{}", stat.removed)),
                        )
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .size(px(12.))
                                .text_color(rgb(palette().faint)),
                        ),
                ),
        );
    }
    rows.into_any_element()
}

impl ConduitApp {
    pub(super) fn tool_group_header(
        &self,
        session_id: u64,
        start: usize,
        _end: usize,
        group_key: String,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = self.chat_rows.group(start).unwrap_or_default();
        let count = group.count;
        let failures = group.failures;
        let running = group.running;
        let elapsed = self
            .session_control
            .runs
            .get(&session_id)
            .map(|run| run.turn_started.elapsed())
            .unwrap_or_default();
        let label = if running {
            conduit::i18n::format(
                "chat.running",
                &[("arg0", format_elapsed(elapsed, false).to_string())],
            )
        } else if failures > 0 {
            conduit::i18n::format(
                "chat.tool_calls_failed",
                &[
                    ("count", format!("{}", count)),
                    ("failures", format!("{}", failures)),
                ],
            )
        } else {
            conduit::i18n::format("chat.tool_calls", &[("count", format!("{}", count))])
        };
        let toggle_key = group_key.clone();
        let can_collapse = !running && group.replied;
        Button::new(SharedString::from(format!(
            "tool-group-{session_id}-{start}"
        )))
        .ghost()
        .debug_selector(move || format!("tool-group-header-{session_id}-{start}"))
        .w_full()
        .h(px(28.))
        .px(px(0.))
        .rounded(px(6.))
        .when(can_collapse, |button| button.cursor_pointer())
        .accessibility_label(format!(
            "{}：{label}",
            if expanded {
                conduit::i18n::text_static("chat.collapse")
            } else {
                conduit::i18n::text_static("chat.expand")
            }
        ))
        .when(can_collapse, |button| {
            button.on_click(cx.listener(move |this, _, _, cx| {
                let key = (session_id, toggle_key.clone());
                if !this.expanded_tool_groups.remove(&key) {
                    this.expanded_tool_groups.insert(key);
                }
                this.sync_chat_rows();
                this.remeasure_message(start);
                cx.notify();
            }))
        })
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    h_flex()
                        .w(px(18.))
                        .h(px(18.))
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .border_1()
                        .border_color(rgb(palette().border))
                        .child(
                            Icon::new(if expanded || running {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .small()
                            .text_color(rgb(palette().faint)),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_left()
                        .text_size(px(12.))
                        .text_color(rgb(palette().muted))
                        .truncate()
                        .child(label),
                ),
        )
        .into_any_element()
    }

    pub(super) fn render_collapsed_tool_group(
        &self,
        session_id: u64,
        start: usize,
        end: usize,
        group_key: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .w_full()
            .px_6()
            .pb_3()
            .child(
                div()
                    .id(SharedString::from(format!(
                        "tool-group-row-{session_id}-{start}"
                    )))
                    .w_full()
                    .min_w_0()
                    .when(!self.store.ui.full_width_content, |row| {
                        row.max_w(px(840.)).mx_auto()
                    })
                    .child(self.tool_group_header(session_id, start, end, group_key, false, cx)),
            )
            .into_any_element()
    }

    /// One-line activity row used for tool calls and reasoning, matching the
    /// compact rows inside "执行工具" groups.
    pub(super) fn activity_row(
        &self,
        session_id: u64,
        index: usize,
        item: &ChatItem,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reasoning = item.kind == ItemKind::Reasoning;
        let detail_key = (session_id, item.key.clone());
        let expanded = if reasoning {
            self.expanded_reasoning.contains(&detail_key)
        } else {
            self.expanded_tools.contains(&detail_key)
        };
        let running = tool_running(item);
        let failed = !running && tool_failed(item);
        let category = if running && reasoning {
            conduit::i18n::text_static("chat.thinking").to_string()
        } else {
            tool_category(item)
        };
        let summary = tool_summary(item);
        let toggle_key = detail_key.clone();
        Button::new(SharedString::from(format!(
            "activity-{session_id}-{}",
            item.key
        )))
        .ghost()
        .debug_selector(move || format!("activity-{session_id}-{index}"))
        .w_full()
        .h(px(28.))
        .px_3()
        .rounded(px(6.))
        .cursor_pointer()
        .accessibility_label(format!(
            "{}{}",
            if expanded {
                conduit::i18n::text_static("chat.collapse")
            } else {
                conduit::i18n::text_static("chat.expand")
            },
            category
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.remeasure_message(index);
            if reasoning {
                if !this.expanded_reasoning.remove(&toggle_key) {
                    this.expanded_reasoning.insert(toggle_key.clone());
                }
            } else if !this.expanded_tools.remove(&toggle_key) {
                this.expanded_tools.insert(toggle_key.clone());
            }
            cx.notify();
        }))
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .child(if running {
                    Spinner::new()
                        .small()
                        .color(rgb(palette().muted).into())
                        .into_any_element()
                } else if failed {
                    Icon::new(IconName::CircleX)
                        .size(px(13.))
                        .text_color(rgb(palette().danger_text))
                        .into_any_element()
                } else {
                    Icon::new(IconName::CircleCheck)
                        .size(px(13.))
                        .text_color(rgb(palette().diff_add_text))
                        .into_any_element()
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(palette().text))
                        .debug_selector(move || format!("activity-category-{session_id}-{index}"))
                        .child(category),
                )
                .when_some(item.approval.as_ref(), |row, approval| {
                    row.child(
                        div()
                            .debug_selector(move || {
                                format!("activity-approval-{session_id}-{index}")
                            })
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(approval.status.label().to_string()),
                    )
                })
                .when(running && !reasoning, |row| {
                    row.child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap(px(3.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().faint))
                                    .child(conduit::i18n::text_static("chat.elapsed")),
                            )
                            .child(
                                Icon::new(FullIconName::Clock3)
                                    .size(px(11.))
                                    .text_color(rgb(palette().faint)),
                            ),
                    )
                })
                .when(failed, |row| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(palette().danger_text))
                            .child(conduit::i18n::text_static("chat.failed")),
                    )
                })
                .when(!summary.is_empty(), |row| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(rgb(palette().faint))
                            .child("·"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(summary),
                    )
                }),
        )
        .into_any_element()
    }

    pub(super) fn render_message(&mut self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let started = Instant::now();
        let index = self.chat_rows.message(row);
        let element = self.render_message_content(index, cx);
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        profiling::record("row_build", started.elapsed());
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_message_render(index, started.elapsed());
            });
        }
        element
    }

    fn render_message_content(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(session) = self
            .store
            .workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .find(|session| Some(session.id) == self.selected_session)
        else {
            return div().into_any_element();
        };
        let Some(item) = session.messages.get(index) else {
            return div().into_any_element();
        };
        if is_empty_chat_placeholder(item) {
            return div()
                .debug_selector(move || format!("empty-message-{index}"))
                .h(px(0.))
                .into_any_element();
        }
        let session_id = session.id;
        let provider = session.provider;
        let group = self.chat_rows.group(index);
        let tool_group = group.map(|group| (group.start, group.end));
        let tool_group_expanded = group.is_some_and(|group| group.expanded);
        let tool_group_running = group.is_some_and(|group| group.running);
        let assistant_replied = group.is_some_and(|group| group.replied);
        if let Some((start, end)) = tool_group
            && !tool_group_expanded
            && !tool_group_running
            && assistant_replied
        {
            if index != start {
                return div().h(px(0.)).into_any_element();
            }
            return self.render_collapsed_tool_group(
                session_id,
                start,
                end,
                session.messages[start].key.clone(),
                cx,
            );
        }
        let typography = self.chat_typography();
        let timestamp = message_timestamp(item.created_at.max(session.created_at));
        let regenerable = item.kind == ItemKind::Assistant
            && self.writable
            && !self.closing
            && !self.session_control.runs.contains_key(&session_id)
            && session.messages[..index]
                .iter()
                .any(|message| message.kind == ItemKind::User)
            && !session.messages[index + 1..]
                .iter()
                .any(|message| matches!(message.kind, ItemKind::User | ItemKind::Assistant));
        let mut element = match item.kind {
            ItemKind::User => {
                let (message_text, references) = file_ref::split_prompt(&item.text);
                let workspace = self.store.workspaces.iter().find(|workspace| {
                    workspace
                        .sessions
                        .iter()
                        .any(|session| session.id == session_id)
                });
                let attachments = h_flex()
                    .id(("user-attachments", index))
                    .debug_selector(|| format!("user-attachments-{index}"))
                    .w_full()
                    .min_w_0()
                    .flex_row_reverse()
                    .flex_wrap()
                    .justify_start()
                    .gap_1()
                    .children(references.iter().enumerate().map(|(file_index, mention)| {
                        let path = Path::new(mention);
                        let link = mention.to_string();
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();
                        let full_path = if path.is_absolute() {
                            display_path(path)
                        } else {
                            workspace
                                .map(|workspace| display_path(&workspace.path.join(path)))
                                .unwrap_or_else(|| mention.to_string())
                        };
                        h_flex()
                            .id(SharedString::from(format!(
                                "user-file-{index}-{file_index}"
                            )))
                            .debug_selector(|| format!("user-file-{index}-{file_index}"))
                            .max_w(px(200.))
                            .h(px(24.))
                            .flex_shrink_0()
                            .min_w_0()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface_strong))
                            .cursor_pointer()
                            .hover(|s| s.bg(rgb(palette().surface_hover)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_chat_file(session_id, &link, window, cx);
                            }))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(full_path.clone())
                                    .build(window, cx)
                            })
                            .child(
                                Icon::new(file_ref::icon_for(path))
                                    .small()
                                    .text_color(rgb(palette().muted)),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_shrink_1()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().muted))
                                    .child(name),
                            )
                    }));
                let copy_id = SharedString::from(format!("user-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let copy_icon = if copied {
                    Icon::new(IconName::Check).text_color(rgb(palette().diff_add_text))
                } else {
                    Icon::new(IconName::Copy)
                };
                let message_group =
                    SharedString::from(format!("user-message-{}-{}", session_id, item.key));
                // Keep user prose literal and selectable; generated file references
                // render below it inside the same bubble without changing the prompt.
                let body = div()
                    .debug_selector(|| format!("user-body-{index}"))
                    // SelectableText does not subscribe this view to its selection
                    // state, so request a repaint as the pointer moves during drag.
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            this.timeline.update(cx, |_, cx| cx.notify());
                            cx.notify();
                        }
                    }))
                    // SelectableText registers drag selection but does not set the
                    // native text cursor like TextView's text renderer does.
                    .cursor_text()
                    .child(gpui_kit::base::SelectableText::new(
                        SharedString::from(format!("user-text-{}-{}", session_id, item.key)),
                        message_text.to_string(),
                    ));
                v_flex()
                    .w_full()
                    .debug_selector(|| format!("user-message-{index}"))
                    .items_end()
                    .gap_1()
                    .group(message_group.clone())
                    .child(
                        v_flex()
                            .id(("user-bubble", index))
                            .debug_selector(|| format!("user-bubble-{index}"))
                            .max_w(px(680.))
                            .min_w_0()
                            .gap_2()
                            .rounded(px(8.))
                            .px_4()
                            .py_3()
                            // A restrained prompt surface distinguishes the user
                            // without turning each turn into a separate card.
                            .bg(rgb(palette().surface_hover))
                            .text_size(px(typography.body_size))
                            .line_height(relative(typography.line_height))
                            .when_some(typography.body_font.clone(), |bubble, family| {
                                bubble.font_family(family)
                            })
                            .text_color(rgb(palette().text))
                            .when(!message_text.is_empty(), |bubble| bubble.child(body))
                            .when(!references.is_empty(), |bubble| bubble.child(attachments)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                h_flex()
                                    .gap(px(2.))
                                    .opacity(0.)
                                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                                    .child(
                                        message_action_button(
                                            copy_id.clone(),
                                            copy_icon,
                                            if copied {
                                                conduit::i18n::text_static("chat.copied")
                                            } else {
                                                conduit::i18n::text_static("chat.copy_message")
                                            },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                let Some(text) = this
                                                    .session(session_id)
                                                    .and_then(|session| session.messages.get(index))
                                                    .map(|item| item.text.clone())
                                                else {
                                                    return;
                                                };
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    text,
                                                ));
                                                this.show_copied(copy_id.clone(), window, cx);
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .opacity(0.)
                                    .group_hover(message_group, |this| this.opacity(1.))
                                    .child(timestamp.clone()),
                            ),
                    )
                    .into_any_element()
            }
            ItemKind::Assistant => {
                let link_app = cx.entity().downgrade();
                let reply_range = assistant_reply_range(session, index);
                let reply_in_progress = reply_range.end == session.messages.len()
                    && (self.session_control.runs.contains_key(&session_id) || session.interrupted);
                let show_actions = !reply_in_progress
                    && !session.messages[index + 1..reply_range.end]
                        .iter()
                        .any(|item| {
                            item.kind == ItemKind::Assistant && !item.text.trim().is_empty()
                        });
                let catalog = self.catalogs.get(&provider);
                let model_id = item
                    .model
                    .as_deref()
                    .or(session.model.as_deref())
                    .or_else(|| catalog.and_then(|catalog| catalog.current.as_deref()));
                let model_label = model_id
                    .map(|id| {
                        catalog
                            .map(|catalog| catalog.label(id))
                            .unwrap_or_else(|| id.into())
                    })
                    .unwrap_or_else(|| conduit::i18n::text_static("chat.cli_default_model").into());
                let copy_id = SharedString::from(format!("reply-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let copy_icon = if copied {
                    Icon::new(IconName::Check).text_color(rgb(palette().diff_add_text))
                } else {
                    Icon::new(IconName::Copy)
                };
                let message_group =
                    SharedString::from(format!("reply-message-{}-{}", session_id, item.key));
                let mut controls = h_flex()
                    .debug_selector(move || format!("reply-controls-{index}"))
                    // Include both hit targets in layout even when metadata is long.
                    .w(px(if regenerable { 45. } else { 19. }))
                    .flex_shrink_0()
                    .gap(px(2.))
                    .child(
                        message_action_button(
                            copy_id.clone(),
                            copy_icon,
                            if copied {
                                conduit::i18n::text_static("chat.copied")
                            } else {
                                conduit::i18n::text_static("chat.copy_message")
                            },
                        )
                        .debug_selector(move || format!("reply-copy-button-{index}"))
                        // The 14px glyph sits centered in a 24px hit target.
                        // Align the glyph with prose while keeping that hit target.
                        .ml(px(-5.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                let Some(session) = this.session(session_id) else {
                                    return;
                                };
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    assistant_reply_text(session, index),
                                ));
                                this.show_copied(copy_id.clone(), window, cx);
                            },
                        )),
                    );
                if regenerable {
                    controls = controls.child(
                        message_action_button(
                            SharedString::from(format!(
                                "reply-regenerate-{}-{}",
                                session_id, item.key
                            )),
                            Icon::new(IconName::RefreshCw),
                            conduit::i18n::text_static("chat.regenerate"),
                        )
                        .debug_selector(move || format!("reply-retry-button-{index}"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.regenerate(index, window, cx),
                        )),
                    );
                }
                let model_meta = h_flex()
                    .debug_selector(move || format!("reply-model-{index}"))
                    .min_w_0()
                    .max_w(px(240.))
                    // Keep short model names at their natural width so the timestamp
                    // follows the label; long names can still shrink and truncate.
                    .flex_shrink_1()
                    .items_center()
                    .gap_1()
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(model_mark(provider, model_id).text_color(rgb(palette().faint)))
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(palette().faint))
                            .min_w_0()
                            .truncate()
                            .child(model_label),
                    );
                let timestamp_meta = div()
                    .debug_selector(move || format!("reply-timestamp-{index}"))
                    .flex_shrink_0()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(timestamp.clone());
                let actions = h_flex()
                    .debug_selector(move || format!("reply-actions-{index}"))
                    .w_full()
                    .min_w_0()
                    .justify_start()
                    .items_center()
                    .gap(px(8.))
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(controls)
                    .child(model_meta)
                    .child(timestamp_meta)
                    .when_some(
                        reply_elapsed_label(
                            session,
                            index,
                            self.session_control.runs.contains_key(&session_id),
                        ),
                        |row, label| {
                            row.child(
                                div()
                                    .debug_selector(move || format!("reply-elapsed-{index}"))
                                    .flex_shrink_0()
                                    .text_size(px(10.))
                                    .text_color(rgb(palette().faint))
                                    .child(label),
                            )
                        },
                    );
                v_flex()
                    .debug_selector(move || format!("assistant-message-{index}"))
                    .w_full()
                    .gap_1()
                    .group(message_group)
                    .child(
                        chat_markdown_view(
                            SharedString::from(format!("reply-{}-{}", session_id, item.key)),
                            item.text.clone(),
                            &typography,
                            &format!("code-copy-{}-{}", session_id, item.key),
                            true,
                        )
                        .on_link_click(move |url, event, window, cx| {
                            let activate = match event {
                                ClickEvent::Mouse(click) => matches!(
                                    click.up.button,
                                    MouseButton::Left | MouseButton::Middle
                                ),
                                ClickEvent::Keyboard(_) => true,
                                ClickEvent::Touch(click) => !click.long_press,
                            };
                            if !activate {
                                return;
                            }
                            let _ = link_app.update(cx, |this, cx| {
                                this.open_chat_link(session_id, url, window, cx);
                            });
                        })
                        .map(|view| {
                            #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
                            {
                                if profiling::enabled() {
                                    return profiling::MeasuredMarkdown(view).into_any_element();
                                }
                                view.into_any_element()
                            }
                            #[cfg(not(all(test, feature = "ui-tests", conduit_shell_desktop)))]
                            {
                                view
                            }
                        }),
                    )
                    .when(show_actions, |message| message.child(actions))
                    .into_any_element()
            }
            ItemKind::Reasoning => {
                let detail_key = (session_id, item.key.clone());
                let expanded = self.expanded_reasoning.contains(&detail_key);
                let mut content = v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(self.activity_row(session_id, index, item, cx));
                if expanded {
                    content = content.child(
                        v_flex()
                            .w_full()
                            .min_w_0()
                            .p_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(rgb(palette().border))
                            .bg(rgb(palette().surface))
                            .text_size(px(typography.body_size))
                            .line_height(relative(typography.line_height))
                            .when_some(typography.body_font.clone(), |body, family| {
                                body.font_family(family)
                            })
                            .text_color(rgb(palette().muted))
                            .child(item.text.clone()),
                    );
                }
                activity_container(content.into_any_element(), tool_group.is_some())
            }
            ItemKind::Tool if is_compact_tool(item) => {
                let detail_key = (session_id, item.key.clone());
                let expanded = self.expanded_tools.contains(&detail_key);
                let mut content = v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_1()
                    .child(self.activity_row(session_id, index, item, cx));
                if expanded {
                    let tool_scroll = self
                        .tool_scrolls
                        .entry((session_id, item.key.clone()))
                        .or_default()
                        .clone();
                    content = content.child(tool_detail_panel(
                        &detail_key,
                        item,
                        &tool_scroll,
                        &typography,
                        &mut self.tool_diffs,
                        &mut self.tool_analysis,
                        cx,
                    ));
                }
                activity_container(content.into_any_element(), tool_group.is_some())
            }
            ItemKind::Tool => {
                let detail_key = (session_id, item.key.clone());
                let expanded = self.expanded_tools.contains(&detail_key);
                let tool_id = format!("tool-{}-{}", session_id, item.key);
                let edit = is_tool_edit(item);
                let edit_paths = if edit {
                    self.tool_analysis
                        .get(item, cx)
                        .map(|value| {
                            value
                                .files
                                .iter()
                                .take(8)
                                .map(|diff| diff.path.clone())
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                let title = if edit {
                    conduit::i18n::text_static("chat.edit_file").to_string()
                } else {
                    item.text.clone()
                };
                let header = Button::new(SharedString::from(format!("{tool_id}-toggle")))
                    .ghost()
                    .w_full()
                    .h(px(36.))
                    .px_3()
                    .cursor_pointer()
                    .accessibility_label(conduit::i18n::format(
                        "chat.tool_call",
                        &[
                            (
                                "arg0",
                                (if expanded {
                                    conduit::i18n::text_static("chat.collapse")
                                } else {
                                    conduit::i18n::text_static("chat.expand")
                                })
                                .to_string(),
                            ),
                            ("arg1", item.text.to_string()),
                        ],
                    ))
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap_2()
                            .when_some(item.approval.as_ref(), |header, approval| {
                                header.child(
                                    div()
                                        .flex_shrink_0()
                                        .px_2()
                                        .py_1()
                                        .rounded(px(5.))
                                        .bg(rgb(palette().surface_strong))
                                        .text_size(px(11.))
                                        .text_color(rgb(match approval.status {
                                            crate::message_status::ApprovalStatus::Allowed => {
                                                palette().diff_add_text
                                            }
                                            crate::message_status::ApprovalStatus::Denied => {
                                                palette().diff_remove_text
                                            }
                                            _ => palette().muted,
                                        }))
                                        .child(approval.status.label().to_string()),
                                )
                            })
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_left()
                                            .text_size(px(13.))
                                            .truncate()
                                            .child(title),
                                    )
                                    .children(edit_paths.iter().take(2).map(|path| {
                                        let name = Path::new(path)
                                            .file_name()
                                            .map(|name| name.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| path.clone());
                                        div()
                                            .flex_shrink_0()
                                            .max_w(px(180.))
                                            .truncate()
                                            .px_1p5()
                                            .py(px(1.))
                                            .rounded(px(4.))
                                            .bg(rgb(palette().surface_strong))
                                            .text_size(px(10.))
                                            .text_color(rgb(palette().muted))
                                            .child(name)
                                    }))
                                    .when(edit_paths.len() > 2, |row| {
                                        row.child(
                                            div()
                                                .flex_shrink_0()
                                                .text_size(px(10.))
                                                .text_color(rgb(palette().faint))
                                                .child(format!("+{}", edit_paths.len() - 2)),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette().faint))
                                    .child(item.status.label().to_string()),
                            )
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .small()
                                .text_color(rgb(palette().faint)),
                            ),
                    )
                    .on_click(cx.listener({
                        let detail_key = detail_key.clone();
                        move |this, _, _, cx| {
                            this.remeasure_message(index);
                            if !this.expanded_tools.remove(&detail_key) {
                                this.expanded_tools.insert(detail_key.clone());
                            }
                            cx.notify();
                        }
                    }));
                let mut card = v_flex()
                    .w_full()
                    .min_w_0()
                    .overflow_hidden()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .child(header);
                if expanded {
                    let tool_scroll = self
                        .tool_scrolls
                        .entry((session_id, item.key.clone()))
                        .or_default()
                        .clone();
                    card = card.child(tool_detail_panel(
                        &detail_key,
                        item,
                        &tool_scroll,
                        &typography,
                        &mut self.tool_diffs,
                        &mut self.tool_analysis,
                        cx,
                    ));
                }
                card.into_any_element()
            }
            ItemKind::Notice => div()
                .text_size(px(11.))
                .text_color(rgb(palette().muted))
                .child(item.text.clone())
                .into_any_element(),
            ItemKind::Error => {
                let raw_error = if item.detail.trim().is_empty() {
                    item.text.clone()
                } else {
                    item.detail.clone()
                };
                let repeats = failure_repeats(item);
                let active = self
                    .session_control
                    .runs
                    .get(&session_id)
                    .is_some_and(|run| item.key.starts_with(&format!("{}-", run.prefix)));
                let recovered = session.messages[index + 1..]
                    .iter()
                    .take_while(|item| item.kind != ItemKind::User)
                    .any(|item| item.kind == ItemKind::Assistant && !item.text.trim().is_empty());
                let title = if active {
                    conduit::i18n::text_static("chat.the_engine_encountered_a_problem_the_task")
                } else if recovered {
                    conduit::i18n::text_static("chat.the_engine_encountered_a_problem_then_resumed")
                } else {
                    failure_title(&item.text)
                };
                let guidance = if active {
                    conduit::i18n::text_static("chat.wait_for_the_current_task_to_finish")
                } else {
                    failure_guidance(&raw_error)
                };
                let summary = failure_summary(&item.text);
                let error_key = (session_id, item.key.clone());
                let expanded = self.expanded_errors.contains(&error_key);
                let copy_text = raw_error.clone();
                let copy_id = SharedString::from(format!("error-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let can_retry = self.writable
                    && !self.closing
                    && !self.session_busy(session_id)
                    && super::runtime::error_retry_prompt(session, index).is_some();
                let mut actions = h_flex().flex_shrink_0().items_center().gap(px(4.));
                actions = actions.child(
                    message_action_button(
                        copy_id.clone(),
                        Icon::new(if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        }),
                        if copied {
                            conduit::i18n::text_static("chat.copied")
                        } else {
                            conduit::i18n::text_static("chat.copy_error_details")
                        },
                    )
                    .debug_selector(|| format!("cli-error-copy-{index}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
                        this.show_copied(copy_id.clone(), window, cx);
                    })),
                );
                if can_retry {
                    actions = actions.child(
                        Button::new(SharedString::from(format!(
                            "error-retry-{}-{}",
                            session_id, item.key
                        )))
                        .ghost()
                        .small()
                        .compact()
                        .cursor_pointer()
                        .child(conduit::i18n::text_static("chat.retry"))
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.retry_error(index, window, cx),
                        )),
                    );
                }
                v_flex()
                    .id(("cli-error", index))
                    .debug_selector(|| format!("cli-error-{index}"))
                    .w_full()
                    .gap_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(rgb(palette().border))
                    .bg(rgb(palette().surface))
                    .p_3()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .small()
                                    .text_color(rgb(palette().error_text)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .font_semibold()
                                    .text_color(rgb(palette().text))
                                    .child(title),
                            )
                            .when(repeats > 1, |row| {
                                row.child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(rgb(palette().muted))
                                        .child(conduit::i18n::format(
                                            "chat.repeated_times",
                                            &[("repeats", format!("{}", repeats))],
                                        )),
                                )
                            })
                            .child(actions),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(relative(1.5))
                            .text_color(rgb(palette().muted))
                            .child(summary),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .line_height(relative(1.4))
                            .text_color(rgb(palette().faint))
                            .child(guidance),
                    )
                    .child(
                        h_flex().child(
                            Button::new(SharedString::from(format!(
                                "error-details-{session_id}-{}",
                                item.key
                            )))
                            .debug_selector(|| format!("cli-error-toggle-{index}"))
                            .ghost()
                            .small()
                            .compact()
                            .cursor_pointer()
                            .icon(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .label(if expanded {
                                conduit::i18n::text_static("chat.hide_details")
                            } else {
                                conduit::i18n::text_static("chat.view_details")
                            })
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.expanded_errors.insert(error_key.clone()) {
                                        this.expanded_errors.remove(&error_key);
                                    }
                                    this.scroll.remeasure();
                                    cx.notify();
                                },
                            )),
                        ),
                    )
                    .when(expanded, |card| {
                        card.child(
                            div()
                                .id(("cli-error-details", index))
                                .debug_selector(|| format!("cli-error-details-{index}"))
                                .max_h(px(200.))
                                .overflow_y_scroll()
                                .rounded(px(6.))
                                .p_3()
                                .bg(rgb(palette().surface_strong))
                                .font_family(typography.code_font.clone())
                                .text_size(px(11.))
                                .line_height(relative(1.5))
                                .text_color(rgb(palette().muted))
                                .cursor_text()
                                .on_mouse_move(cx.listener(|_, event: &MouseMoveEvent, _, cx| {
                                    if event.pressed_button == Some(MouseButton::Left) {
                                        cx.notify();
                                    }
                                }))
                                .child(gpui_kit::base::SelectableText::new(
                                    SharedString::from(format!(
                                        "error-text-{session_id}-{}",
                                        item.key
                                    )),
                                    raw_error,
                                )),
                        )
                    })
                    .into_any_element()
            }
        };
        if let Some((start, end)) = tool_group
            && (tool_group_expanded || tool_group_running)
            && index == start
        {
            let group_key = session.messages[start].key.clone();
            let header = self.tool_group_header(session_id, start, end, group_key, true, cx);
            element = v_flex()
                .w_full()
                .gap_1()
                .child(header)
                .child(element)
                .into_any_element();
        }
        let highlighted = self
            .message_focus
            .as_ref()
            .is_some_and(|(id, key, _)| *id == session_id && key == &item.key);
        let compact_activity = item.kind == ItemKind::Reasoning || is_compact_tool(item);
        // One rhythm for the whole transcript: 12px between blocks, dense inside
        // an expanded tool group. `activity_container` already contributes 2px.
        let row = div()
            .debug_selector(move || format!("message-row-{index}"))
            .w_full()
            .px_6()
            .when(compact_activity, |row| {
                row.pb(if tool_group.is_some() {
                    px(0.)
                } else {
                    px(10.)
                })
            })
            .when(!compact_activity, |row| row.pb_3())
            .child(
                div()
                    .id(SharedString::from(format!(
                        "message-{}-{}",
                        session_id, item.key
                    )))
                    .w_full()
                    .min_w_0()
                    .when(!self.store.ui.full_width_content, |card| {
                        card.max_w(px(840.)).mx_auto()
                    })
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if highlighted {
                        rgb(palette().focus).into()
                    } else {
                        transparent_black()
                    })
                    .when(highlighted, |row| row.bg(rgb(palette().surface_hover)))
                    .child(element)
                    .map(|card| {
                        // Observe actual paint in both dev and release probes. This
                        // transparent wrapper has no production layout/state cost.
                        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
                        {
                            use gpui_kit::TestSupportExt;
                            card.test_support()
                        }
                        #[cfg(not(all(test, feature = "ui-tests", conduit_shell_desktop)))]
                        {
                            card
                        }
                    }),
            );
        row.into_any_element()
    }
}
