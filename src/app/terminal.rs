use super::*;
use crate::terminal::{RunState, Runtime, ShellProfile};
use conduit::plugins::context::WorkspaceKey;
use gpui_kit::component::badge::Badge;
#[cfg(all(test, feature = "ui-tests"))]
mod tests;
mod view;

struct Session {
    id: u64,
    workspace: WorkspaceKey,
    profile: ShellProfile,
    environment: Option<crate::environments::Environment>,
    runtime: Arc<Runtime>,
    view: Entity<view::TerminalView>,
    last_state: RunState,
    close_requested: bool,
}
#[derive(Default)]
pub(super) struct TerminalUi {
    sessions: Vec<Session>,
    selected: HashMap<WorkspaceKey, u64>,
    next_id: u64,
    pub open: bool,
    maximized: bool,
    return_focus: Option<FocusHandle>,
    pub close_confirmed: bool,
    confirm_close: Option<u64>,
    feedback: Option<String>,
}
impl TerminalUi {
    pub fn forget_workspace(&mut self, path: &Path) {
        self.sessions.retain(|s| s.workspace.path() != path);
        self.selected.retain(|k, _| k.path() != path);
    }
    pub fn active(&self) -> bool {
        self.sessions.iter().any(|s| s.runtime.active())
    }
    pub fn workspace_busy(&self, path: &Path) -> bool {
        self.sessions
            .iter()
            .any(|s| s.workspace.path() == path && s.runtime.active())
    }
    pub fn stop_all(&self) {
        for s in &self.sessions {
            s.runtime.stop();
        }
    }
    pub fn cleanup_error(&self) -> Option<String> {
        self.sessions.iter().find_map(|s| match s.runtime.state() {
            RunState::CleanupFailed(e) => Some(e),
            _ => None,
        })
    }
    fn current(&self, path: &Path) -> Option<&Session> {
        let key = WorkspaceKey::from_host_path(path.into()).ok()?;
        let id = self.selected.get(&key)?;
        self.sessions
            .iter()
            .find(|s| s.id == *id && s.workspace == key)
    }
}

pub(super) fn init(cx: &mut App) {
    // Suppress global/composer actions while still delivering raw terminal keys.
    for key in [
        "ctrl-c",
        "ctrl-v",
        "ctrl-w",
        "ctrl-e",
        "ctrl-p",
        "ctrl-k",
        "ctrl-m",
        "ctrl-,",
        "ctrl-l",
        "ctrl-a",
        "ctrl-shift-c",
        "ctrl-shift-v",
        "shift-insert",
        "enter",
        "tab",
        "escape",
    ] {
        cx.bind_keys([KeyBinding::new(key, NoAction, Some("ConduitTerminal"))]);
    }
}
impl ConduitApp {
    pub(super) fn terminal_command(
        &mut self,
        create: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use conduit::plugins::builtins;
        self.sync_plugin_context();
        if let Some(command) = self.plugins.host_command(if create {
            builtins::NEW_TERMINAL
        } else {
            builtins::TOGGLE_TERMINAL
        }) {
            self.dispatch_plugin_command(&command, window, cx);
        }
    }
    fn terminal_key(&self) -> Option<WorkspaceKey> {
        WorkspaceKey::from_host_path(self.selected_workspace_path()?).ok()
    }
    pub(super) fn terminal_focused(&self, window: &Window, cx: &App) -> bool {
        self.selected_workspace_path()
            .and_then(|p| self.terminals.current(&p))
            .is_some_and(|s| s.view.read(cx).focus.is_focused(window))
    }
    fn remember_terminal_focus(&mut self, window: &Window, cx: &App) {
        let in_terminal = self
            .terminals
            .sessions
            .iter()
            .any(|s| s.view.read(cx).focus.is_focused(window));
        if !in_terminal {
            self.terminals.return_focus = window.focused(cx);
        }
    }
    pub(super) fn confirm_terminal_exit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.terminals.active() || self.terminals.close_confirmed || self.updater.exiting {
            return false;
        }
        let app = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let app = app.clone();
            dialog
                .title(conduit::i18n::text_static(
                    "terminal.terminate_terminals_and_exit",
                ))
                .content(|content, _, _| {
                    content.child(conduit::i18n::text_static(
                        "terminal.terminals_are_still_running_exiting_terminates_all",
                    ))
                })
                .on_ok(move |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.terminals.close_confirmed = true;
                        app.request_close(window, cx);
                    });
                    true
                })
        });
        true
    }
    pub(super) fn new_terminal(
        &mut self,
        profile: ShellProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch_terminal(profile, None, window, cx);
    }

    pub(super) fn environment_terminal(
        &mut self,
        environment: crate::environments::Environment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_workspace_path().as_ref() != Some(&environment.workspace) {
            return;
        }
        self.launch_terminal(ShellProfile::Auto, Some(environment), window, cx);
    }

    fn launch_terminal(
        &mut self,
        profile: ShellProfile,
        environment: Option<crate::environments::Environment>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable || self.closing || self.updater.preparing() {
            return;
        }
        let Some(key) = self.terminal_key() else {
            self.terminals.feedback = Some(
                conduit::i18n::text_static("terminal.select_a_workspace_to_open_a_terminal").into(),
            );
            self.terminals.open = true;
            cx.notify();
            return;
        };
        if self.terminals.sessions.len() >= 8
            || self
                .terminals
                .sessions
                .iter()
                .filter(|s| s.workspace == key)
                .count()
                >= 4
        {
            self.terminals.feedback = Some(
                conduit::i18n::text_static("terminal.up_to_4_terminals_per_workspace_and").into(),
            );
            self.terminals.open = true;
            cx.notify();
            return;
        }
        self.remember_terminal_focus(window, cx);
        self.terminals.close_confirmed = false;
        self.terminals.next_id += 1;
        let id = self.terminals.next_id;
        let runtime = Arc::new(if let Some(environment) = &environment {
            match environment.command() {
                Ok((exe, args)) => Runtime::command(exe, args, key.path().into()),
                Err(error) => {
                    self.terminals.feedback = Some(format!("{error:#}"));
                    cx.notify();
                    return;
                }
            }
        } else {
            Runtime::start(profile, key.path().into())
        });
        let view = cx.new(|cx| view::TerminalView::new(runtime.clone(), cx));
        view.read(cx).focus.clone().focus(window, cx);
        self.terminals.selected.insert(key.clone(), id);
        self.terminals.sessions.push(Session {
            id,
            workspace: key,
            profile,
            environment,
            runtime,
            view,
            last_state: RunState::Starting,
            close_requested: false,
        });
        self.terminals.open = true;
        self.terminals.feedback = None;
        self.terminals.confirm_close = None;
        cx.notify();
    }
    pub(super) fn toggle_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminals.open && self.terminal_focused(window, cx) {
            self.hide_terminal(window, cx);
            return;
        }
        self.terminals.open = true;
        self.remember_terminal_focus(window, cx);
        if let Some(session) = self
            .selected_workspace_path()
            .and_then(|p| self.terminals.current(&p))
        {
            session.view.read(cx).focus.clone().focus(window, cx);
            cx.notify();
        } else {
            self.new_terminal(self.store.ui.terminal_profile, window, cx);
        }
    }
    pub(super) fn hide_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.terminals.open = false;
        self.terminals.confirm_close = None;
        for s in &self.terminals.sessions {
            s.view.update(cx, |v, _| v.blur());
        }
        self.leave_terminal(window, cx);
        cx.notify();
    }
    pub(super) fn leave_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(focus) = self.terminals.return_focus.take() {
            focus.focus(window, cx);
        } else {
            self.input.read(cx).focus_handle(cx).focus(window, cx);
        }
    }
    pub(super) fn cycle_terminal(
        &mut self,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = self.terminal_key() else {
            return;
        };
        let ids: Vec<_> = self
            .terminals
            .sessions
            .iter()
            .filter(|s| s.workspace == key)
            .map(|s| s.id)
            .collect();
        if ids.is_empty() {
            return;
        }
        let index = ids
            .iter()
            .position(|id| self.terminals.selected.get(&key) == Some(id))
            .unwrap_or(0);
        let id = ids[(index + if back { ids.len() - 1 } else { 1 }) % ids.len()];
        self.select_terminal(id, window, cx);
    }
    fn select_terminal(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.terminal_key() else {
            return;
        };
        for s in &self.terminals.sessions {
            s.view.update(cx, |v, _| v.blur());
        }
        if let Some(s) = self
            .terminals
            .sessions
            .iter()
            .find(|s| s.id == id && s.workspace == key)
        {
            self.terminals.selected.insert(key, id);
            s.view.read(cx).focus.clone().focus(window, cx);
            self.terminals.confirm_close = None;
            cx.notify();
        }
    }
    fn close_terminal(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.terminal_key() else {
            return;
        };
        let Some(s) = self
            .terminals
            .sessions
            .iter_mut()
            .find(|s| s.id == id && s.workspace == key)
        else {
            return;
        };
        if s.runtime.active() && self.terminals.confirm_close != Some(id) {
            self.terminals.confirm_close = Some(id);
            cx.notify();
            return;
        }
        s.close_requested = true;
        s.runtime.stop();
        self.terminals.confirm_close = None;
        self.poll_terminals(window, cx);
        cx.notify();
    }
    fn restart_terminal(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.writable || self.closing || self.updater.preparing() {
            return;
        }
        let Some(key) = self.terminal_key() else {
            return;
        };
        let Some(s) = self
            .terminals
            .sessions
            .iter()
            .find(|s| s.id == id && s.workspace == key && !s.runtime.active())
        else {
            return;
        };
        let profile = s.profile;
        let environment = s.environment.clone();
        self.terminals.sessions.retain(|s| s.id != id);
        self.launch_terminal(profile, environment, window, cx);
    }
    pub(super) fn poll_terminals(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.terminal_key();
        let visible = self.terminals.open && !self.settings_ui.open && self.startup.is_none();
        let mut changed = false;
        if let Some(id) = self.terminals.confirm_close
            && (!visible
                || !self.terminals.sessions.iter().any(|s| {
                    s.id == id && key.as_ref() == Some(&s.workspace) && s.runtime.active()
                }))
        {
            self.terminals.confirm_close = None;
            changed = true;
        }
        for s in &mut self.terminals.sessions {
            let state = s.runtime.state();
            if state != s.last_state {
                s.last_state = state;
                changed = true;
            }
            let selected = visible
                && key.as_ref() == Some(&s.workspace)
                && self.terminals.selected.get(&s.workspace) == Some(&s.id);
            if selected && s.runtime.dirty.swap(false, Ordering::AcqRel) {
                s.view.update(cx, |_, cx| cx.notify());
            }
            if !selected {
                if s.view.read(cx).focus.is_focused(window) {
                    self.input.read(cx).focus_handle(cx).focus(window, cx);
                }
                s.view.update(cx, |v, _| v.blur());
            }
            s.view.update(cx, |v, _| {
                v.input_enabled =
                    selected && self.writable && !self.closing && !self.updater.preparing()
            });
        }
        let old = self.terminals.sessions.len();
        let removed_focus = self.terminals.sessions.iter().any(|s| {
            s.close_requested && !s.runtime.active() && s.view.read(cx).focus.is_focused(window)
        });
        self.terminals
            .sessions
            .retain(|s| !s.close_requested || s.runtime.active());
        changed |= old != self.terminals.sessions.len();
        self.terminals.selected.retain(|key, id| {
            if self
                .terminals
                .sessions
                .iter()
                .any(|s| s.id == *id && &s.workspace == key)
            {
                true
            } else if let Some(s) = self
                .terminals
                .sessions
                .iter()
                .rev()
                .find(|s| &s.workspace == key)
            {
                *id = s.id;
                true
            } else {
                false
            }
        });
        if removed_focus {
            if let Some(session) = self
                .selected_workspace_path()
                .and_then(|p| self.terminals.current(&p))
            {
                session.view.read(cx).focus.clone().focus(window, cx);
            } else {
                self.leave_terminal(window, cx);
            }
        }
        if self.closing
            && let Some(error) = self.terminals.cleanup_error()
        {
            self.closing = false;
            self.terminals.close_confirmed = false;
            self.message = Some(conduit::i18n::format(
                "terminal.exit_paused_because_terminal_cleanup_is_incomplete",
                &[("error", error.to_string())],
            ));
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
    pub(super) fn terminal_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.selected_workspace_path().map_or(0, |p| {
            self.terminals
                .sessions
                .iter()
                .filter(|s| s.workspace.path() == p)
                .count()
        });
        div()
            .flex_shrink_0()
            .pr_1()
            .child(
                Badge::new().count(count).child(
                    Button::new("toggle-terminal")
                        .debug_selector(|| "toggle-terminal".into())
                        .ghost()
                        .small()
                        .compact()
                        .icon(FullIconName::SquareTerminal)
                        .selected(self.terminals.open)
                        .tooltip(conduit::i18n::format(
                            "terminal.toggle_terminal_in_this_workspace_ctrl_backtick",
                            &[("count", format!("{}", count))],
                        ))
                        .accessibility_label(conduit::i18n::format(
                            "terminal.open_workspace_terminal_terminals",
                            &[("count", format!("{}", count))],
                        ))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.terminal_command(false, window, cx)
                        })),
                ),
            )
            .into_any_element()
    }
    pub(super) fn with_terminal_panel(
        &mut self,
        content: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if !self.terminals.open {
            return content;
        }
        let panel = self.render_terminal_panel(cx);
        if self.terminals.maximized || f32::from(window.viewport_size().height) < 510. {
            return panel;
        }
        let state = window.use_keyed_state("terminal-split", cx, |_, _| ResizableState::default());
        let height = self
            .store
            .ui
            .terminal_height
            .clamp(180., 800.)
            .min(f32::from(window.viewport_size().height) * 0.5);
        let app = cx.entity();
        v_resizable("terminal-split-group")
            .with_state(&state)
            .child(
                resizable_panel()
                    .size_range(px(200.)..px(5000.))
                    .child(content),
            )
            .child(
                resizable_panel()
                    .size(px(height))
                    .size_range(px(180.)..px(800.))
                    .child(panel),
            )
            .on_resize(move |state, _, cx| {
                let height = state.read(cx).sizes().get(1).copied();
                if let Some(height) = height {
                    app.update(cx, |app, _| {
                        app.store.ui.terminal_height = f32::from(height).clamp(180., 800.);
                        app.dirty = true;
                    });
                }
            })
            .into_any_element()
    }
    fn render_terminal_panel(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key = self.terminal_key();
        let ids: Vec<_> = self
            .terminals
            .sessions
            .iter()
            .filter(|s| Some(&s.workspace) == key.as_ref())
            .map(|s| s.id)
            .collect();
        let selected = key
            .as_ref()
            .and_then(|k| self.terminals.selected.get(k))
            .copied();
        let mut tabs = TabBar::new("terminal-tabs").small().min_w_0().flex_1();
        for id in ids {
            let s = self.terminals.sessions.iter().find(|s| s.id == id).unwrap();
            let title = format!(
                "{} {}{}",
                s.environment
                    .as_ref()
                    .map_or(s.profile.label(), |e| e.name.as_str()),
                id,
                if s.runtime.active() {
                    ""
                } else {
                    conduit::i18n::text_static("terminal.exited")
                }
            );
            let close_selector = format!("terminal-tab-close-{id}");
            let confirming = self.terminals.confirm_close == Some(id) && s.runtime.active();
            let close_label = if confirming {
                conduit::i18n::format(
                    "terminal.click_again_to_terminate_and_close_including",
                    &[("title", title.to_string())],
                )
            } else if s.runtime.active() {
                conduit::i18n::format(
                    "terminal.terminate_and_close_click_again_to_confirm",
                    &[("title", title.to_string())],
                )
            } else {
                conduit::i18n::format("terminal.close", &[("title", title.to_string())])
            };
            tabs =
                tabs.child(
                    Tab::new()
                        .debug_selector(move || format!("terminal-tab-{id}"))
                        .label(title)
                        .selected(selected == Some(id))
                        .suffix(
                            h_flex().items_center().pl_0p5().pr_1().child(
                                Button::new(SharedString::from(close_selector.clone()))
                                    .debug_selector(move || close_selector.clone())
                                    .ghost()
                                    .xsmall()
                                    .compact()
                                    .icon(
                                        Icon::new(if confirming {
                                            FullIconName::CircleX
                                        } else {
                                            FullIconName::X
                                        })
                                        .when(confirming, |icon| {
                                            icon.text_color(rgb(palette().danger_text))
                                        }),
                                    )
                                    .tooltip(close_label.clone())
                                    .accessibility_label(close_label)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.close_terminal(id, window, cx);
                                        cx.stop_propagation();
                                    })),
                            ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_terminal(id, window, cx)
                        })),
                );
        }
        let app = cx.entity();
        let can_create =
            self.writable && !self.closing && !self.updater.preparing() && key.is_some();
        let header = h_flex()
            .h(px(34.))
            .items_center()
            .gap_1()
            .px_2()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(palette().border))
            .child(
                div()
                    .text_size(ui_text_size(12.))
                    .child(conduit::i18n::text_static("shortcuts.terminal")),
            )
            .child(tabs)
            .child(
                Button::new("terminal-attach-selection")
                    .small()
                    .compact()
                    .ghost()
                    .icon(FullIconName::MessageSquarePlus)
                    .tooltip(conduit::i18n::text_static("artifact.terminal_attach"))
                    .accessibility_label(conduit::i18n::text_static("artifact.terminal_attach"))
                    .disabled(!can_create)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let Some(workspace) = this.selected_workspace_path() else {
                            return;
                        };
                        let Some(session) = this.terminals.current(&workspace) else {
                            return;
                        };
                        let Some(text) = session
                            .runtime
                            .selected_text()
                            .filter(|text| !text.trim().is_empty())
                        else {
                            return;
                        };
                        if text.len() > 64 * 1024 {
                            this.terminals.feedback =
                                Some(conduit::i18n::text("workflow.context_limit"));
                            cx.notify();
                            return;
                        }
                        let source = session.environment.as_ref().map_or_else(
                            || session.profile.label().to_string(),
                            |e| format!("{}:{}", e.host, e.directory),
                        );
                        let context = conduit::i18n::format(
                            "artifact.terminal_context",
                            &[
                                ("workspace", workspace.display().to_string()),
                                ("source", source),
                                ("text", text),
                            ],
                        );
                        if let Err(error) = this.append_review_draft(
                            &workspace,
                            this.selected_session,
                            None,
                            &context,
                            window,
                            cx,
                        ) {
                            this.terminals.feedback = Some(error);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-new")
                    .debug_selector(|| "terminal-new".into())
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Plus)
                    .tooltip(conduit::i18n::text_static(
                        "terminal.new_terminal_ctrl_shift_backtick",
                    ))
                    .accessibility_label(conduit::i18n::text_static("shortcuts.new_terminal"))
                    .disabled(!can_create)
                    .on_click(cx.listener(|this, _, w, cx| this.terminal_command(true, w, cx))),
            )
            .child(
                Button::new("terminal-shell")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronDown)
                    .tooltip(conduit::i18n::text_static("terminal.select_shell"))
                    .accessibility_label(conduit::i18n::text_static("terminal.select_shell"))
                    .disabled(!can_create)
                    .dropdown_menu(move |menu, _, _| {
                        let mut menu = menu;
                        for profile in [
                            ShellProfile::Auto,
                            ShellProfile::PowerShell,
                            ShellProfile::Cmd,
                        ] {
                            let app = app.clone();
                            menu = menu.item(PopupMenuItem::new(profile.label()).on_click(
                                move |_, w, cx| {
                                    app.update(cx, |app, cx| {
                                        app.store.ui.terminal_profile = profile;
                                        app.dirty = true;
                                        app.new_terminal(profile, w, cx);
                                    });
                                },
                            ));
                        }
                        menu
                    }),
            )
            .child(
                Button::new("terminal-maximize")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::Maximize)
                    .tooltip(conduit::i18n::text_static(
                        "terminal.maximize_restore_terminal",
                    ))
                    .accessibility_label(conduit::i18n::text_static(
                        "terminal.maximize_or_restore_terminal",
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.terminals.maximized = !this.terminals.maximized;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("terminal-hide")
                    .ghost()
                    .small()
                    .compact()
                    .icon(FullIconName::ChevronDown)
                    .tooltip(conduit::i18n::text_static(
                        "terminal.collapse_panel_and_keep_processes_running",
                    ))
                    .accessibility_label(conduit::i18n::text_static("terminal.collapse_terminal"))
                    .on_click(cx.listener(|this, _, w, cx| this.hide_terminal(w, cx))),
            );
        let mut panel = v_flex()
            .id("terminal-panel")
            .debug_selector(|| "terminal-panel".into())
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(rgb(palette().canvas))
            .child(header);
        if let Some(error) = &self.terminals.feedback {
            panel = panel.child(
                div()
                    .px_2()
                    .text_size(ui_text_size(12.))
                    .text_color(rgb(palette().danger_text))
                    .child(error.clone()),
            );
        }
        if let Some(s) = selected.and_then(|id| self.terminals.sessions.iter().find(|s| s.id == id))
        {
            let id = s.id;
            let status = s.runtime.state();
            let active = status.active();
            let family = self
                .store
                .ui
                .code_font_family
                .clone()
                .map(SharedString::from)
                .unwrap_or_else(|| self.default_mono_font.clone());
            s.view.update(cx, |v, cx| {
                v.input_enabled = self.writable && !self.closing && !self.updater.preparing();
                v.configure(family, self.store.ui.code_font_size, cx)
            });
            let view = s.view.clone();
            if status != RunState::Running {
                panel = panel.child(
                    h_flex()
                        .h(px(28.))
                        .px_2()
                        .gap_2()
                        .items_center()
                        .flex_shrink_0()
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_size(ui_text_size(11.))
                                .text_color(rgb(palette().muted))
                                .child(status.label()),
                        )
                        .when(!active, |row| {
                            row.child(
                                Button::new("terminal-restart")
                                    .ghost()
                                    .xsmall()
                                    .label(conduit::i18n::text_static("terminal.restart"))
                                    .disabled(!can_create)
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.restart_terminal(id, w, cx)
                                    })),
                            )
                        }),
                );
            }
            panel = panel.child(
                div()
                    .p_2()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(view.cached(StyleRefinement::default().size_full())),
            );
        } else {
            panel = panel.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(if key.is_some() {
                        conduit::i18n::text_static("terminal.no_terminals_in_this_workspace")
                    } else {
                        conduit::i18n::text_static("terminal.select_a_workspace_to_open_a_terminal")
                    })
                    .child(
                        Button::new("terminal-create-first")
                            .label(conduit::i18n::text_static("shortcuts.new_terminal"))
                            .disabled(!can_create)
                            .on_click(
                                cx.listener(|this, _, w, cx| this.terminal_command(true, w, cx)),
                            ),
                    ),
            );
        }
        panel.into_any_element()
    }
}
