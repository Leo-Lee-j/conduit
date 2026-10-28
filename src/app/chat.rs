//! Transcript rows, message actions, and turn statistics.

use super::*;

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
        format!("{}分{:02}秒", seconds / 60, seconds % 60)
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
        return Some(format!(
            "耗时 {}",
            format_elapsed(Duration::from_millis(elapsed), true)
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
    Some(format!(
        "耗时约 {}",
        format_elapsed(Duration::from_secs(elapsed), false)
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

    /// Floating summary above the composer: plan progress and the file churn
    /// of the latest turn. Clicking it reveals the step and file details.
    pub(super) fn turn_stats_pill(
        &self,
        session_id: u64,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session = self.session(session_id)?;
        let plan = self
            .plans
            .get(&session_id)
            .cloned()
            .filter(|steps| !steps.is_empty())
            .unwrap_or_else(|| plan_from_messages(session));
        let stats = turn_file_stats(session);
        let live = self.runs.contains_key(&session_id);
        let run_activity = self.runs.get(&session_id).map(|run| {
            let model = run
                .model
                .as_deref()
                .map(|model| {
                    self.catalogs
                        .get(&session.provider)
                        .map_or_else(|| model.to_string(), |catalog| catalog.label(model))
                })
                .unwrap_or_else(|| "CLI 默认模型".into());
            (
                run.activity.clone(),
                run.turn_started.elapsed(),
                model,
                if session.provider == Provider::OpenCode {
                    run.opencode_variant
                        .clone()
                        .unwrap_or_else(|| "默认".into())
                } else {
                    run.reasoning_effort.label().to_owned()
                },
            )
        });
        if plan.is_empty() && stats.files.is_empty() && run_activity.is_none() {
            return None;
        }
        let completed = plan
            .iter()
            .filter(|step| step.status == PlanStatus::Completed)
            .count();
        let total = plan.len();
        let files = stats.files.len();
        let open = self.stats_details;
        let mut pill = h_flex()
            .id("turn-stats")
            .debug_selector(|| "turn-stats".into())
            .h(px(30.))
            .px_3()
            .gap_2()
            .items_center()
            .rounded_full()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .shadow_sm()
            .cursor_pointer()
            .hover(|style| style.bg(rgb(palette().surface_hover)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.stats_details = !this.stats_details;
                cx.notify();
            }))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronLeft
                })
                .size(px(13.))
                .text_color(rgb(palette().faint)),
            );
        if total > 0 {
            pill = pill.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(palette().muted))
                    .child(format!("步骤 {completed} / {total}")),
            );
        }
        if total > 0 && files > 0 {
            pill = pill.child(
                div()
                    .text_size(px(10.))
                    .text_color(rgb(palette().faint))
                    .child("·"),
            );
        }
        if files > 0 {
            pill = pill
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().muted))
                        .child(format!("{files} 个文件已修改")),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().diff_add_text))
                        .child(format!("+{}", stats.added)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(palette().diff_remove_text))
                        .child(format!("-{}", stats.removed)),
                );
        }
        let root = self
            .store
            .workspaces
            .iter()
            .find(|workspace| workspace.sessions.iter().any(|item| item.id == session_id))
            .map(|workspace| workspace.path.to_string_lossy().into_owned());
        let mut status_row = h_flex()
            .id("turn-status-row")
            .debug_selector(|| "turn-status-row".into())
            .w_full()
            .min_w_0()
            .when(!self.store.ui.full_width_content, |row| {
                row.max_w(px(840.)).mx_auto()
            })
            .items_center()
            .when(!live, |row| row.justify_center())
            .gap_2();
        if let Some((activity, elapsed, model, effort)) = run_activity {
            status_row = status_row.child(
                h_flex()
                    .id("engine-activity")
                    .debug_selector(|| "engine-activity".into())
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
                            .child(activity),
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
                            .child(format_elapsed(elapsed, false)),
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
                            .child(format!("思考强度：{effort}")),
                    ),
            );
        }
        status_row = status_row.when(live, |row| row.child(div().flex_1()));
        if total > 0 || files > 0 {
            status_row = status_row.child(pill.flex_shrink_0());
        }
        let mut column = v_flex().w_full().items_center().pb(px(8.));
        if open && (total > 0 || files > 0) {
            column = column
                .child(self.turn_stats_details(&plan, &stats, root.as_deref(), live))
                .child(div().h(px(8.)));
        }
        Some(column.child(status_row).into_any_element())
    }

    pub(super) fn turn_stats_details(
        &self,
        plan: &[PlanStep],
        stats: &TurnFileStats,
        root: Option<&str>,
        live: bool,
    ) -> AnyElement {
        let mut rows = v_flex()
            .id("turn-stats-details")
            .w_full()
            .min_w_0()
            .when(!self.store.ui.full_width_content, |rows| {
                rows.max_w(px(840.)).mx_auto()
            })
            .max_h(px(240.))
            .overflow_y_scroll()
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
        for (path, stat) in &stats.files {
            rows = rows.child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::File)
                            .size(px(12.))
                            .text_color(rgb(palette().faint)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(workspace_relative_path(path, root)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().diff_add_text))
                            .child(format!("+{}", stat.added)),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(palette().diff_remove_text))
                            .child(format!("-{}", stat.removed)),
                    ),
            );
        }
        rows.into_any_element()
    }

    pub(super) fn tool_group_header(
        &self,
        session_id: u64,
        start: usize,
        end: usize,
        group_key: String,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = self.session(session_id).map_or(0, |session| {
            session.messages[start..end]
                .iter()
                .filter(|item| is_groupable_activity(item))
                .count()
        });
        let (failures, running) = self
            .session(session_id)
            .map(|session| {
                let items = &session.messages[start..end.min(session.messages.len())];
                (
                    items.iter().filter(|item| tool_failed(item)).count(),
                    items.iter().any(tool_running),
                )
            })
            .unwrap_or((0, false));
        let elapsed = self
            .runs
            .get(&session_id)
            .map(|run| run.turn_started.elapsed())
            .unwrap_or_default();
        let label = if running {
            format!("正在执行中 · {}", format_elapsed(elapsed, false))
        } else if failures > 0 {
            format!("执行工具 {count} 次，其中 {failures} 次失败")
        } else {
            format!("执行工具 {count} 次")
        };
        let toggle_key = group_key.clone();
        let can_collapse = !running
            && self
                .session(session_id)
                .is_some_and(|session| tool_group_has_reply(session, end));
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
            if expanded { "收起" } else { "展开" }
        ))
        .when(can_collapse, |button| {
            button.on_click(cx.listener(move |this, _, _, cx| {
                let key = (session_id, toggle_key.clone());
                if !this.expanded_tool_groups.remove(&key) {
                    this.expanded_tool_groups.insert(key);
                }
                this.scroll.remeasure_items(start..end);
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
            "思考中".to_string()
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
            if expanded { "收起" } else { "展开" },
            category
        ))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.scroll.remeasure_items(index..index + 1);
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
                            .child(approval.status.clone()),
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
                                    .child("已运行"),
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
                            .child("运行失败"),
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

    pub(super) fn render_message(&mut self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let started = Instant::now();
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
        let tool_group = contiguous_tool_group(session, index);
        let tool_group_expanded = tool_group.is_some_and(|(start, _)| {
            self.expanded_tool_groups
                .contains(&(session_id, session.messages[start].key.clone()))
        });
        // Keep the tool group open through the whole tool phase. Collapse only
        // after the assistant has started a visible reply, unless the user
        // pinned the group open.
        let tool_group_running = tool_group
            .is_some_and(|(start, end)| session.messages[start..end].iter().any(tool_running));
        let assistant_replied =
            tool_group.is_some_and(|(_, end)| tool_group_has_reply(session, end));
        let tool_group_expanded = tool_group_expanded || !assistant_replied;
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
            && !self.runs.contains_key(&session_id)
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
                let copy_text = item.text.clone();
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
                    .on_mouse_move(cx.listener(|_, event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
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
                            .rounded(px(16.))
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
                                            if copied { "已复制" } else { "复制消息" },
                                        )
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                cx.write_to_clipboard(ClipboardItem::new_string(
                                                    copy_text.clone(),
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
                let reply_range = assistant_reply_range(session, index);
                let reply_in_progress = reply_range.end == session.messages.len()
                    && (self.runs.contains_key(&session_id) || session.interrupted);
                let show_actions = !reply_in_progress
                    && !session.messages[index + 1..reply_range.end]
                        .iter()
                        .any(|item| {
                            item.kind == ItemKind::Assistant && !item.text.trim().is_empty()
                        });
                let copy_text = if show_actions {
                    assistant_reply_text(session, index)
                } else {
                    String::new()
                };
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
                    .unwrap_or_else(|| "CLI 默认模型".into());
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
                            if copied { "已复制" } else { "复制消息" },
                        )
                        .debug_selector(move || format!("reply-copy-button-{index}"))
                        // The 14px glyph sits centered in a 24px hit target.
                        // Align the glyph with prose while keeping that hit target.
                        .ml(px(-5.))
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()));
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
                            "重新生成",
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
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(
                        model_mark(provider, model_id)
                            .size(px(11.))
                            .text_color(rgb(palette().faint)),
                    )
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
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .opacity(0.)
                    .group_hover(message_group.clone(), |this| this.opacity(1.))
                    .child(controls)
                    .child(model_meta)
                    .child(timestamp_meta)
                    .when_some(
                        reply_elapsed_label(session, index, self.runs.contains_key(&session_id)),
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
                    .child(chat_markdown_view(
                        SharedString::from(format!("reply-{}-{}", session_id, item.key)),
                        item.text.clone(),
                        &typography,
                        &format!("code-copy-{}-{}", session_id, item.key),
                        true,
                    ))
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
                            .text_color(rgb(palette().muted))
                            .child(item.text.clone()),
                    );
                }
                activity_container(content.into_any_element(), tool_group.is_some())
            }
            ItemKind::Tool if is_compact_tool(item) => {
                let detail_key = (session_id, item.key.clone());
                let expanded = self.expanded_tools.contains(&detail_key);
                let tool_id = format!("tool-{}-{}", session_id, item.key);
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
                        &tool_id,
                        item,
                        &tool_scroll,
                        &typography,
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
                    edit_tool_diffs(item)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|diff| diff.path)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let title = if edit {
                    "编辑文件".to_string()
                } else {
                    item.text.clone()
                };
                let header = Button::new(SharedString::from(format!("{tool_id}-toggle")))
                    .ghost()
                    .w_full()
                    .h(px(36.))
                    .px_3()
                    .cursor_pointer()
                    .accessibility_label(format!(
                        "{}工具调用：{}",
                        if expanded { "收起" } else { "展开" },
                        item.text
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
                                        .text_color(rgb(match approval.status.as_str() {
                                            "已允许" => palette().diff_add_text,
                                            "已拒绝" => palette().diff_remove_text,
                                            _ => palette().muted,
                                        }))
                                        .child(approval.status.clone()),
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
                                    .child(item.status.clone()),
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.scroll.remeasure_items(index..index + 1);
                        if !this.expanded_tools.remove(&detail_key) {
                            this.expanded_tools.insert(detail_key.clone());
                        }
                        cx.notify();
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
                        &tool_id,
                        item,
                        &tool_scroll,
                        &typography,
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
                let summary = failure_summary(&item.text);
                let error_key = (session_id, item.key.clone());
                let expanded = self.expanded_errors.contains(&error_key);
                let copy_text = raw_error.clone();
                let copy_id = SharedString::from(format!("error-copy-{}-{}", session_id, item.key));
                let copied = self.copied_action.as_ref() == Some(&copy_id);
                let can_retry = self.writable
                    && !self.closing
                    && !self.runs.contains_key(&session_id)
                    && session.native_id.is_some()
                    && session.messages[..index]
                        .iter()
                        .any(|message| message.kind == ItemKind::User)
                    && !session.messages[index + 1..].iter().any(|message| {
                        matches!(message.kind, ItemKind::User | ItemKind::Assistant)
                    });
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
                            "已复制"
                        } else {
                            "复制错误详情"
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
                        .child("重试")
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
                                    .child("本次回复未完成"),
                            )
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
                                "收起详情"
                            } else {
                                "查看详情"
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
                    .child(element),
            );
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_message_render(index, started.elapsed());
            });
        }
        row.into_any_element()
    }
}
