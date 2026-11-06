//! Notification presentation and navigation. Business workflows publish through `notify_user`.
use super::plugin_ui::right::{RightBody, RightView};
use super::*;
use crate::notifications::{
    Center, Level, Lifecycle, Notice, Presentation, Record, Source, Target, history::History,
};
use conduit::plugins::{builtins, ids::ViewId, state::ViewInstanceKey};

mod approvals;
use approvals::ApprovalBinding;
pub(super) use approvals::approval_message;
use gpui_kit::component::{
    alert::Alert,
    badge::Badge,
    notification::{Notification, NotificationDelivery},
};

struct AppToast;
struct OverflowToast;

pub(super) struct NotificationUi {
    pub center: Center,
    history: History,
    pending: Vec<u64>,
    visible: HashMap<u64, u64>,
    banners: BTreeMap<u64, Record>,
    overflow: usize,
    overflow_shown: usize,
    pub unread_only: bool,
    pub scroll: ScrollHandle,
    return_focus: Option<FocusHandle>,
    focus: FocusHandle,
    pub view: Option<Entity<NotificationPanel>>,
    rendered_revision: u64,
    last_prune: Instant,
    shutdown: Option<Instant>,
    pub update_state: Option<String>,
    persistence_error: Option<String>,
    load_failed: bool,
    target_error: Option<(u64, String)>,
    // Capabilities for live requests only. Never written to notification history.
    approvals: HashMap<u64, ApprovalBinding>,
}

impl NotificationUi {
    pub(super) fn new(writable: bool, cx: &mut Context<ConduitApp>) -> Self {
        let path = Store::path().with_file_name("notifications.json");
        // UI fixtures never share notification history or acquire the production writer lock.
        #[cfg(test)]
        let path = {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            path.with_file_name(format!(
                "notifications-ui-{}.json",
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
        };
        let (center, history) = History::open(path, writable && !cfg!(test), unix_now());
        Self {
            center,
            history,
            pending: vec![],
            visible: HashMap::new(),
            banners: BTreeMap::new(),
            overflow: 0,
            overflow_shown: 0,
            unread_only: false,
            scroll: ScrollHandle::new(),
            return_focus: None,
            focus: cx.focus_handle(),
            view: None,
            rendered_revision: 0,
            last_prune: Instant::now(),
            shutdown: None,
            update_state: None,
            persistence_error: None,
            load_failed: !writable,
            target_error: None,
            approvals: HashMap::new(),
        }
    }
    pub(super) fn finish(&mut self) -> bool {
        let started = *self.shutdown.get_or_insert_with(Instant::now);
        self.history.submit(&self.center, true);
        self.history.poll();
        !self.history.busy() || started.elapsed() >= Duration::from_millis(750)
    }
}

impl Drop for NotificationUi {
    fn drop(&mut self) {
        self.history.submit(&self.center, true);
    }
}

/// Cached between notification changes unless accessibility needs a fresh subtree.
pub(super) struct NotificationPanel {
    app: WeakEntity<ConduitApp>,
    _theme: Subscription,
}
impl Render for NotificationPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.app
            .update(cx, |app, cx| app.notification_list(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

fn level_icon(level: Level) -> FullIconName {
    match level {
        Level::Info => FullIconName::Info,
        Level::Success => FullIconName::CircleCheck,
        Level::Warning => FullIconName::TriangleAlert,
        Level::Error => FullIconName::CircleX,
    }
}
fn level_color(level: Level) -> u32 {
    match level {
        Level::Info => palette().muted,
        Level::Success => palette().diff_add_text,
        Level::Warning => palette().activity_running,
        Level::Error => palette().danger_text,
    }
}

impl ConduitApp {
    /// Unified entry point: records once, then schedules presentation on the UI thread.
    pub(super) fn notify_user(&mut self, request: Notice, cx: &mut Context<Self>) -> u64 {
        let published = self.notifications.center.publish(request, unix_now());
        if published.changed {
            if published.presentation == Presentation::Banner {
                if let Some(record) = self.notifications.center.get(published.id).cloned() {
                    self.notifications.banners.insert(published.id, record);
                    while self.notifications.banners.len() > crate::notifications::MAX_RECORDS {
                        self.notifications.banners.pop_first();
                    }
                }
            } else {
                self.notifications.banners.remove(&published.id);
            }
            if published.presentation == Presentation::Toast && !self.closing {
                self.notifications.pending.retain(|id| *id != published.id);
                self.notifications.pending.push(published.id);
                // The queue is bounded independently of history retention.
                if self.notifications.pending.len() > crate::notifications::MAX_RECORDS {
                    self.notifications.pending.remove(0);
                    self.notifications.overflow = self.notifications.overflow.saturating_add(1);
                }
            }
            self.refresh_notifications(cx);
        }
        published.id
    }
    pub(super) fn refresh_notifications(&mut self, cx: &mut Context<Self>) {
        self.expire_notification_approvals();
        self.notifications.rendered_revision = self.notifications.center.revision();
        if let Some(view) = &self.notifications.view {
            view.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }
    pub(super) fn session_notice_target(&self, id: u64) -> Option<Target> {
        self.store.workspaces.iter().find_map(|workspace| {
            workspace
                .sessions
                .iter()
                .find(|session| session.id == id)
                .map(|session| Target::Session {
                    workspace: workspace.path.clone(),
                    session: id,
                    created_at: session.created_at,
                    native_id: session.native_id.clone(),
                })
        })
    }
    fn notification_session_index(&self, target: &Target) -> Option<usize> {
        let Target::Session {
            workspace,
            session,
            created_at,
            native_id,
        } = target
        else {
            return None;
        };
        self.store.workspaces.iter().position(|candidate| {
            candidate.path == *workspace
                && candidate.sessions.iter().any(|candidate| {
                    candidate.id == *session
                        && candidate.created_at == *created_at
                        && native_id
                            .as_ref()
                            .is_none_or(|expected| candidate.native_id.as_ref() == Some(expected))
                })
        })
    }
    pub(super) fn session_is_visible(&self, id: u64) -> bool {
        self.selected_session == Some(id)
            && self.active_file.is_none()
            && self.active_diff.is_none()
            && !self.branch_navigation_open()
    }
    pub(super) fn resolve_notification(&mut self, key: &str, cx: &mut Context<Self>) {
        let before = self.notifications.center.revision();
        self.notifications.center.resolve(key, unix_now());
        let banners = self.notifications.banners.len();
        self.notifications
            .banners
            .retain(|_, record| record.key.as_deref() != Some(key));
        if before != self.notifications.center.revision()
            || banners != self.notifications.banners.len()
        {
            self.refresh_notifications(cx);
        }
    }
    pub(super) fn toggle_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plugins.right_is(builtins::NOTIFICATIONS) {
            self.close_right_view(window, cx);
        } else {
            self.open_right_view(&ViewId::new(builtins::NOTIFICATIONS).unwrap(), window, cx);
        }
    }
    pub(super) fn activate_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.notifications.return_focus = window.focused(cx);
        self.notifications.focus.focus(window, cx);
        self.refresh_notifications(cx);
    }
    pub(super) fn deactivate_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.notifications.return_focus.take() {
            focus.focus(window, cx);
        }
    }
    pub(super) fn notification_bell(&self, cx: &mut Context<Self>) -> AnyElement {
        let unread = self.notifications.center.unread();
        div()
            .flex_shrink_0()
            .pr_1()
            .child(
                Badge::new().count(unread).max(99).child(
                    Button::new("notification-bell")
                        .debug_selector(|| "notification-bell".into())
                        .ghost()
                        .small()
                        .compact()
                        .icon(FullIconName::Bell)
                        .selected(self.plugins.right_is(builtins::NOTIFICATIONS))
                        .tooltip(format!("通知 · {unread} 条未读"))
                        .accessibility_label(format!("通知，{unread} 条未读"))
                        .on_click(
                            cx.listener(|app, _, window, cx| app.toggle_notifications(window, cx)),
                        ),
                ),
            )
            .into_any_element()
    }
    pub(super) fn render_notification_view(
        &mut self,
        _: &ViewInstanceKey,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> RightView {
        let app = cx.weak_entity();
        let view = self
            .notifications
            .view
            .get_or_insert_with(|| {
                cx.new(|cx| NotificationPanel {
                    app,
                    _theme: cx.observe_global::<Theme>(|_, cx| cx.notify()),
                })
            })
            .clone();
        // gpui-pre 0.3.9 replays cached drawing and input handlers, but not the
        // accessibility subtree. Rebuild it while assistive technology is active
        // so history rows and their actions remain available on every frame.
        let view = if window.is_a11y_active() {
            view.into_any_element()
        } else {
            view.cached(StyleRefinement::default().size_full())
                .into_any_element()
        };
        let body = div()
            .id("notification-panel")
            .test_support()
            .debug_selector(|| "notification-panel".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .child(view);
        let content = v_flex()
            .size_full()
            .track_focus(&self.notifications.focus)
            .on_key_down(cx.listener(|app, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    app.close_right_view(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(body);
        RightBody::Content(content.into_any_element()).into()
    }
    fn notification_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let unread_only = self.notifications.unread_only;
        let records: Vec<_> = self
            .notifications
            .center
            .records()
            .iter()
            .filter(|record| !unread_only || record.read_at.is_none())
            .collect();
        let mut list = v_flex()
            .id("notification-list")
            .role(Role::List)
            .aria_label(if unread_only {
                "未读通知"
            } else {
                "通知历史"
            })
            .debug_selector(|| "notification-list".into())
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .track_scroll(&self.notifications.scroll);
        if records.is_empty() {
            list = list.child(
                v_flex()
                    .py_8()
                    .gap_2()
                    .items_center()
                    .child(Icon::new(FullIconName::Bell).text_color(rgb(palette().faint)))
                    .child(
                        div()
                            .id("notifications-empty")
                            .role(Role::Label)
                            .aria_value(if unread_only {
                                "没有未读通知"
                            } else {
                                "暂无通知"
                            })
                            .text_sm()
                            .text_color(rgb(palette().muted))
                            .child(if unread_only {
                                "没有未读通知"
                            } else {
                                "暂无通知"
                            }),
                    ),
            );
        }
        for record in records {
            list = list.child(self.notification_row(record, cx));
        }
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        Button::new("notifications-all")
                            .small()
                            .ghost()
                            .label("全部")
                            .selected(!unread_only)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.notifications.unread_only = false;
                                app.refresh_notifications(cx);
                            })),
                    )
                    .child(
                        Button::new("notifications-unread")
                            .small()
                            .ghost()
                            .label("未读")
                            .selected(unread_only)
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.notifications.unread_only = true;
                                app.refresh_notifications(cx);
                            })),
                    )
                    .child(
                        Button::new("notifications-read-all")
                            .debug_selector(|| "notifications-read-all".into())
                            .small()
                            .ghost()
                            .label("全部标为已读")
                            .disabled(self.notifications.center.unread() == 0)
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.notifications.center.mark_all_read(unix_now());
                                app.dismiss_read_toasts(window, cx);
                                app.refresh_notifications(cx);
                            })),
                    )
                    .child(
                        Button::new("notifications-clear-read")
                            .debug_selector(|| "notifications-clear-read".into())
                            .small()
                            .ghost()
                            .label("清除已读")
                            .on_click(cx.listener(|app, _, window, cx| {
                                app.notifications.center.clear_read();
                                app.dismiss_read_toasts(window, cx);
                                app.refresh_notifications(cx);
                            })),
                    ),
            )
            .when_some(self.notifications.history.error.clone(), |body, error| {
                // The pinned Alert has no accessible-name builder. Name its
                // group so the persistent diagnostic can be read on Windows.
                body.child(
                    div()
                        .id("notification-history-diagnostic")
                        .role(Role::Group)
                        .aria_label(error.clone())
                        .child(Alert::warning("notification-history-error", error).small()),
                )
            })
            .child(list)
            .into_any_element()
    }
    fn notification_row(&self, record: &Record, cx: &mut Context<Self>) -> AnyElement {
        use gpui_kit::base::TestSupportExt as _;
        let id = record.id;
        let unread = record.read_at.is_none();
        let approval = self.notification_approval_state(id);
        let group = SharedString::from(format!("notification-card-{id}"));
        let time = DateTime::from_timestamp(record.updated_at as i64, 0)
            .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
            .unwrap_or_default();
        let full_time = DateTime::from_timestamp(record.updated_at as i64, 0)
            .map(|time| {
                time.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S %:z")
                    .to_string()
            })
            .unwrap_or_default();
        let location = match &record.target {
            Some(Target::Session { workspace, .. }) | Some(Target::Workspace(workspace)) => {
                format!(" · {}", display_path(workspace))
            }
            _ => String::new(),
        };
        let lifecycle = if let Some(outcome) = record.approval_outcome {
            format!(" · {}", outcome.label())
        } else {
            match record.lifecycle {
                Lifecycle::Active => " · 待处理",
                Lifecycle::Resolved => " · 已结束",
                Lifecycle::Recorded => "",
            }
            .into()
        };
        let count = if record.occurrences > 1 {
            format!(" · {} 次", record.occurrences)
        } else {
            String::new()
        };
        v_flex()
            .id(("notification-record", id))
            .group(group.clone())
            .role(Role::ListItem)
            .aria_label(format!(
                "{} · {}",
                record.title,
                if unread { "未读" } else { "已读" }
            ))
            .debug_selector(move || format!("notification-record-{id}"))
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .p_3()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(palette().border))
            .bg(rgb(palette().surface))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        h_flex()
                            .debug_selector(move || format!("notification-icon-{id}"))
                            .size(px(16.))
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(level_icon(record.level))
                                    .small()
                                    .text_color(rgb(level_color(record.level))),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("notification-title-{id}"))
                            .min_w_0()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .child(record.title.clone()),
                    )
                    .when(unread, |row| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(11.))
                                .text_color(rgb(palette().text))
                                .child("未读"),
                        )
                    })
                    .when(!unread && record.lifecycle != Lifecycle::Active, |row| {
                        row.child(
                            Button::new(("notification-remove", id))
                                .debug_selector(move || format!("notification-remove-{id}"))
                                .self_start()
                                .small()
                                .compact()
                                .ghost()
                                .icon(FullIconName::X)
                                .opacity(0.)
                                .group_hover(group.clone(), |style| style.opacity(1.))
                                .focus_visible(|style| style.opacity(1.))
                                .tooltip("移除通知")
                                .accessibility_label("移除通知")
                                .on_click(cx.listener(move |app, _, window, cx| {
                                    app.notifications.center.remove_read(id);
                                    app.dismiss_read_toasts(window, cx);
                                    app.refresh_notifications(cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .id(("notification-message", id))
                    .role(Role::Label)
                    .aria_value(record.message.clone())
                    .min_w_0()
                    .overflow_x_hidden()
                    .text_size(px(12.))
                    .text_color(rgb(palette().text))
                    .child(record.message.clone()),
            )
            .child(
                div()
                    .id(("notification-source", id))
                    .role(Role::Label)
                    .aria_value(format!(
                        "{}{location}{lifecycle}{count}",
                        record.source.label()
                    ))
                    .text_size(px(10.))
                    .text_color(rgb(palette().muted))
                    .child(format!(
                        "{}{location}{lifecycle}{count}",
                        record.source.label()
                    )),
            )
            .when(
                self.notifications
                    .target_error
                    .as_ref()
                    .is_some_and(|(target, _)| *target == id),
                |row| {
                    row.child(
                        div()
                            .id(("notification-target-error", id))
                            .role(Role::Label)
                            .aria_value(self.notifications.target_error.as_ref().unwrap().1.clone())
                            .text_size(px(11.))
                            .text_color(rgb(palette().muted))
                            .child(self.notifications.target_error.as_ref().unwrap().1.clone()),
                    )
                },
            )
            .child(
                h_flex()
                    .debug_selector(move || format!("notification-footer-{id}"))
                    .w_full()
                    .min_w_0()
                    .h(px(24.))
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .id(("notification-time", id))
                            .test_support()
                            .debug_selector(move || format!("notification-time-{id}"))
                            .role(Role::Label)
                            .aria_value(full_time.clone())
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(rgb(palette().muted))
                            .opacity(0.)
                            .group_hover(group, |style| style.opacity(1.))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(full_time.clone())
                                    .build(window, cx)
                            })
                            .child(time),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_1()
                            .when_some(approval, |row, (allow_once, submitted)| {
                                row.child(
                                    Button::new(("notification-allow", id))
                                        .debug_selector(move || format!("notification-allow-{id}"))
                                        .small()
                                        .ghost()
                                        .label(if submitted {
                                            "提交中…"
                                        } else {
                                            "允许本次"
                                        })
                                        .accessibility_label(if submitted {
                                            "审批提交中"
                                        } else {
                                            "允许本次审批"
                                        })
                                        .tooltip(if allow_once {
                                            "仅允许本次请求"
                                        } else {
                                            "此请求不支持仅允许本次，请查看会话"
                                        })
                                        .disabled(submitted || !allow_once)
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.decide_notification(id, true, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("notification-deny", id))
                                        .debug_selector(move || format!("notification-deny-{id}"))
                                        .small()
                                        .ghost()
                                        .label("拒绝")
                                        .accessibility_label("拒绝本次审批")
                                        .disabled(submitted)
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.decide_notification(id, false, window, cx);
                                        })),
                                )
                            })
                            .when(unread && approval.is_none(), |row| {
                                row.child(
                                    Button::new(("notification-read", id))
                                        .debug_selector(move || format!("notification-read-{id}"))
                                        .small()
                                        .ghost()
                                        .label("标为已读")
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.read_notification(id, window, cx)
                                        })),
                                )
                            })
                            .when(record.target.is_some(), |row| {
                                row.child(
                                    Button::new(("notification-open", id))
                                        .debug_selector(move || format!("notification-open-{id}"))
                                        .small()
                                        .ghost()
                                        .label("查看")
                                        .on_click(cx.listener(move |app, _, window, cx| {
                                            app.open_notification(id, window, cx)
                                        })),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }
    fn read_notification(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.notifications.center.mark_read(id, unix_now());
        self.dismiss_read_toasts(window, cx);
        self.refresh_notifications(cx);
    }
    fn open_notification(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing {
            return;
        }
        let target = self
            .notifications
            .center
            .get(id)
            .and_then(|record| record.target.clone());
        self.read_notification(id, window, cx);
        self.notifications.target_error = None;
        let valid = match target {
            Some(target @ Target::Session { session, .. }) => {
                let index = self.notification_session_index(&target);
                if let Some(index) = index {
                    window.close_all_dialogs(cx);
                    if self.settings_ui.open {
                        self.close_settings(window, cx);
                    }
                    self.switch(index, Some(session), window, cx);
                    true
                } else {
                    false
                }
            }
            Some(Target::Workspace(path)) => {
                if let Some(index) = self
                    .store
                    .workspaces
                    .iter()
                    .position(|workspace| workspace.path == path)
                {
                    window.close_all_dialogs(cx);
                    if self.settings_ui.open {
                        self.close_settings(window, cx);
                    }
                    self.switch(index, None, window, cx);
                    self.show_git_navigation(cx);
                    true
                } else {
                    false
                }
            }
            Some(Target::Settings) => {
                window.close_all_dialogs(cx);
                self.open_settings(window, cx);
                true
            }
            Some(Target::Updates) => {
                window.close_all_dialogs(cx);
                self.open_about_dialog(window, cx);
                true
            }
            None => true,
        };
        if !valid {
            self.notifications.target_error = Some((id, "目标已不存在".into()));
            self.open_right_view(&ViewId::new(builtins::NOTIFICATIONS).unwrap(), window, cx);
        }
        self.refresh_notifications(cx);
    }
    fn dismiss_read_toasts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remove: Vec<_> = self
            .notifications
            .visible
            .keys()
            .copied()
            .filter(|id| {
                self.notifications.center.get(*id).is_none_or(|record| {
                    record.read_at.is_some() || record.lifecycle == Lifecycle::Resolved
                })
            })
            .collect();
        for id in remove {
            self.notifications.visible.remove(&id);
            window.remove_notification1::<AppToast>(id, cx);
        }
        if self.notifications.center.unread() == 0 && self.notifications.overflow_shown > 0 {
            self.notifications.overflow = 0;
            self.notifications.overflow_shown = 0;
            window.remove_notification::<OverflowToast>(cx);
        }
    }
    pub(super) fn notification_banner(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = v_flex().w_full().flex_shrink_0();
        for record in self.notifications.banners.values().rev().take(3) {
            let id = record.id;
            let message = format!("{}：{}", record.title, record.message);
            let alert = match record.level {
                Level::Info => Alert::info(("notification-banner", id), message),
                Level::Success => Alert::success(("notification-banner", id), message),
                Level::Warning => Alert::warning(("notification-banner", id), message),
                Level::Error => Alert::error(("notification-banner", id), message),
            }
            .banner()
            .small();
            let alert = if record.lifecycle != Lifecycle::Active {
                alert.on_close(cx.listener(move |app, _, _, cx| {
                    app.notifications.banners.remove(&id);
                    cx.notify();
                }))
            } else {
                alert
            };
            rows = rows.child(alert);
        }
        rows.into_any_element()
    }
    pub(super) fn poll_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expire_notification_approvals();
        let persistence_error = if self.notifications.load_failed {
            Some("应用数据无法加载，原文件已保留；当前为只读模式。".to_string())
        } else {
            self.persistence.error.clone()
        };
        if persistence_error != self.notifications.persistence_error {
            if let Some(error) = &persistence_error {
                self.notify_user(
                    Notice::new(
                        Source::Persistence,
                        Level::Error,
                        "数据保存不可用",
                        error.clone(),
                    )
                    .key(
                        "application-persistence",
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                            .to_string(),
                    )
                    .active()
                    .presentation(Presentation::Banner),
                    cx,
                );
            } else {
                if self
                    .notifications
                    .persistence_error
                    .as_ref()
                    .is_some_and(|error| {
                        self.message.as_deref() == Some(format!("保存失败：{error}").as_str())
                    })
                {
                    self.message = None;
                }
                self.resolve_notification("application-persistence", cx);
            }
            self.notifications.persistence_error = persistence_error;
        }
        let stale: Vec<_> = self
            .notifications
            .center
            .records()
            .iter()
            .filter(|record| record.lifecycle == Lifecycle::Active)
            .filter_map(|record| match &record.target {
                Some(target @ Target::Session { .. })
                    if self.notification_session_index(target).is_none() =>
                {
                    record.key.clone()
                }
                _ => None,
            })
            .collect();
        for key in stale {
            self.resolve_notification(&key, cx);
        }
        let center = &self.notifications.center;
        self.notifications.banners.retain(|id, _| {
            center
                .get(*id)
                .is_none_or(|record| record.lifecycle != Lifecycle::Resolved)
        });
        if self.notifications.last_prune.elapsed() >= Duration::from_secs(60) {
            self.notifications.center.prune(unix_now());
            self.notifications.last_prune = Instant::now();
        }
        if self.notifications.history.poll()
            || self.notifications.rendered_revision != self.notifications.center.revision()
        {
            self.refresh_notifications(cx);
        }
        self.notifications
            .history
            .submit(&self.notifications.center, self.closing);
        self.dismiss_read_toasts(window, cx);
        if self.closing || self.startup.is_some() {
            return;
        }
        for id in std::mem::take(&mut self.notifications.pending) {
            let Some(record) = self.notifications.center.get(id).cloned() else {
                continue;
            };
            if record.read_at.is_some() || record.lifecycle == Lifecycle::Resolved {
                continue;
            }
            if !self.notifications.visible.contains_key(&id)
                && self.notifications.visible.len() >= 2
            {
                self.notifications.overflow = self.notifications.overflow.saturating_add(1);
                continue;
            }
            let revision = record.revision;
            self.notifications.visible.insert(id, revision);
            let view = cx.weak_entity();
            let close_view = view.clone();
            let mut preview: String = record.message.chars().take(180).collect();
            if preview.len() < record.message.len() {
                preview.push('…');
            }
            let action_view = view.clone();
            let has_target = record.target.is_some();
            let note = match record.level {
                Level::Info => Notification::info(preview),
                Level::Success => Notification::success(preview),
                Level::Warning => Notification::warning(preview),
                Level::Error => Notification::error(preview),
            }
            .id1::<AppToast>(id)
            .title(record.title)
            .delivery(NotificationDelivery::InApp)
            .on_click(move |_, window, cx| {
                let _ = view.update(cx, |app, cx| app.open_notification(id, window, cx));
            })
            .on_close(move |_, cx| {
                let _ = close_view.update(cx, |app, _| {
                    if app.notifications.visible.get(&id) == Some(&revision) {
                        app.notifications.visible.remove(&id);
                    }
                });
            });
            let note = if has_target {
                note.action(move |_, _, _| {
                    let action_view = action_view.clone();
                    Button::new(("notification-action", id))
                        .small()
                        .label("查看")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            let _ = action_view
                                .update(cx, |app, cx| app.open_notification(id, window, cx));
                        })
                })
                .autohide(true)
            } else {
                note
            };
            window.push_notification(note, cx);
        }
        if self.notifications.overflow > self.notifications.overflow_shown {
            self.notifications.overflow_shown = self.notifications.overflow;
            let count = self.notifications.center.unread();
            let view = cx.weak_entity();
            window.push_notification(
                Notification::info(format!("通知中心有 {count} 条未读消息，点击查看"))
                    .id::<OverflowToast>()
                    .delivery(NotificationDelivery::InApp)
                    .on_click(move |_, window, cx| {
                        let _ = view.update(cx, |app, cx| {
                            if !(app.plugins.right_is(builtins::NOTIFICATIONS)) {
                                app.toggle_notifications(window, cx);
                            }
                        });
                    }),
                cx,
            );
        }
    }
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
