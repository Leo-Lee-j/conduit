//! Window layout, sidebar, conversation, composer, and toolbar.

use super::*;

mod cache;
mod chat_approval;
mod conversation;
mod scroll;
mod timeline_source;
pub(in crate::app) use timeline_source::TimelineSource;

#[cfg(all(test, feature = "ui-tests"))]
mod tests;

// All navigation modes share one range and one retained resize state.
pub(super) const NAVIGATION_MIN_WIDTH: f32 = 200.;
pub(super) const NAVIGATION_MAX_WIDTH: f32 = 420.;
pub(super) const NAVIGATION_DEFAULT_WIDTH: f32 = NAVIGATION_MAX_WIDTH;

#[derive(Default)]
struct ContentMotionState {
    route: Option<String>,
    generation: u64,
    animate: bool,
}

/// Keep wheel-driven invalidation inside the transcript. The list notifies its
/// owning view on every wheel event, even when no surrounding controls change.
pub(super) struct ChatTimeline {
    app: WeakEntity<ConduitApp>,
    scroll: ListState,
    wheel: std::rc::Rc<std::cell::RefCell<scroll::Wheel>>,
    render_state: Option<cache::State>,
    source: TimelineSource,
}

impl ChatTimeline {
    pub(super) fn wheel_pending(&self, app: &ConduitApp) -> bool {
        self.wheel
            .borrow()
            .is_pending(app, &self.source, &self.scroll)
    }

    pub(super) fn resume_follow(&self, app: &ConduitApp) -> bool {
        self.wheel
            .borrow()
            .resume_follow(app, &self.source, &self.scroll)
    }

    pub(super) fn new(app: WeakEntity<ConduitApp>, scroll: ListState) -> Self {
        Self {
            app,
            scroll,
            wheel: Default::default(),
            render_state: None,
            source: TimelineSource::Main,
        }
    }

    pub(in crate::app) fn for_session(
        app: WeakEntity<ConduitApp>,
        scroll: ListState,
        source: TimelineSource,
    ) -> Self {
        Self {
            source,
            ..Self::new(app, scroll)
        }
    }

    fn handle_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        origin: ListOffset,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.)).y;
        if delta == px(0.) {
            // Horizontal/zero-delta packets do not leave the vertical anchor.
            self.scroll.scroll_to(origin);
            return;
        }
        if let Some(app) = self.app.upgrade() {
            self.wheel.borrow_mut().sync(app.read(cx), &self.source);
        }
        let at_bottom = self.wheel.borrow_mut().apply(&self.scroll, origin, delta);
        let _ = self.app.update(cx, |app, cx| {
            self.source.scrolled(&self.scroll, at_bottom, app, cx);
        });
        cx.notify();
    }
}

impl Render for ChatTimeline {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(app) = self.app.upgrade() {
            let app = app.read(cx);
            if self.source.following(app)
                && !self.source.navigating(app)
                && !self.wheel_pending(app)
            {
                self.scroll.scroll_to_end();
            }
            let monitor = app
                .store
                .ui
                .performance_monitor
                .then(|| app.performance.clone());
            if let Some(monitor) = monitor {
                monitor.update(cx, |monitor, _| monitor.begin_message_frame());
            }
        }
        let app = self.app.clone();
        let source = self.source.clone();
        let list = gpui_kit::list(self.scroll.clone(), move |index, window, cx| {
            app.update(cx, |app, cx| source.render(app, index, window, cx).0)
                .unwrap_or_else(|_| div().into_any_element())
        })
        .size_full();
        let origin = std::rc::Rc::new(std::cell::Cell::new(self.scroll.logical_scroll_top()));
        let capture_origin = origin.clone();
        let scroll = self.scroll.clone();
        let layout_scroll = self.scroll.clone();
        let layout_app = self.app.clone();
        let wheel = self.wheel.clone();
        let source = self.source.clone();
        div()
            .relative()
            .size_full()
            .pt(px(5.))
            .on_scroll_wheel(cx.listener(move |this, event, _, cx| {
                this.handle_scroll(event, origin.get(), cx);
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        wheel.borrow_mut().prepare(
                            &layout_scroll,
                            &layout_app,
                            &source,
                            bounds,
                            window,
                            cx,
                        );
                    },
                    move |bounds, _, window, _| {
                        // gpui-pre 0.3.9 coalesces wheel packets against the
                        // last painted anchor, losing movement on reversals.
                        // Capture the live logical anchor before its listener,
                        // then restore it in our bubble handler. Nested
                        // scrollers get first refusal and can stop bubbling.
                        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, _, _| {
                            if phase == DispatchPhase::Capture && bounds.contains(&event.position) {
                                capture_origin.set(scroll.logical_scroll_top());
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(list)
    }
}

impl ConduitApp {
    /// The window's first frame: shown until the initial CLI history scan
    /// merges, so the main layout never renders a half-imported workspace
    /// list. The title bar stays live so the window can still be dragged,
    /// moved, and closed while loading.
    fn startup_page(&self) -> AnyElement {
        let title_bar = TitleBar::new()
            .bg(chrome_background(palette().canvas))
            .border_color(rgb(palette().border))
            .child(
                h_flex().items_center().gap_2().child(app_icon(20.)).child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_semibold()
                        .text_color(rgb(palette().text))
                        .child("Conduit"),
                ),
            );
        let workspaces = self.store.workspaces.len();
        v_flex()
            .id("startup-loading")
            .debug_selector(|| "startup-loading".into())
            .relative()
            .size_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .child(title_bar)
            .child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(app_icon(64.))
                    .child(
                        div()
                            .text_size(ui_text_size(20.))
                            .font_semibold()
                            .child("Conduit"),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Spinner::new().small().color(rgb(palette().loading).into()))
                            .child(
                                div()
                                    .text_size(ui_text_size(13.))
                                    .text_color(rgb(palette().muted))
                                    .child(conduit::i18n::format(
                                        "shell.loading_cli_history_for_workspaces",
                                        &[("workspaces", format!("{}", workspaces))],
                                    )),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ConduitApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.release_hidden_background_browser(cx);
        // The startup page owns the window until the initial history scan
        // merges, so the workspace sidebar never renders a half-imported
        // list.
        if self.startup.is_some() {
            return self.startup_page();
        }
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let _host_render = chat::profiling::host_span("host_render");
        {
            #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
            let _span = chat::profiling::host_span("host_context");
            self.sync_plugin_context();
            self.refresh_plugin_views(cx);
        }
        self.ensure_selected_transcript(cx);
        let render_started = Instant::now();
        let selected = self
            .selected_session
            .and_then(|id| self.session(id))
            .map(SessionSummary::from);
        let message_count = selected.as_ref().map_or(0, |session| session.message_count);
        let content_route = if self.settings_ui.open {
            format!("settings:{}", self.settings_ui.page)
        } else if let Some(id) = self.github_ui.active {
            format!("github:{id}")
        } else if let Some(id) = self.usage_ui.active {
            format!("usage:{id}")
        } else if let Some(id) = self.active_diff {
            format!("diff:{id}")
        } else if let Some(id) = self.active_file {
            format!("file:{id}")
        } else if let Some(route) = self.subagent_route() {
            format!("child:{}:{}:{}", route.root, route.run, route.agent)
        } else if let Some(session) = &selected {
            format!("chat:{}:{}", session.id, session.message_count == 0)
        } else {
            format!("workspace:{:?}", self.selected_workspace)
        };
        let content_motion = window.use_keyed_state("main-content-motion", cx, |_, _| {
            ContentMotionState::default()
        });
        let content_motion_generation = content_motion.update(cx, |state, _| {
            let route_without_transition = |route: &str| {
                route.starts_with("file:")
                    || route.starts_with("diff:")
                    || route.starts_with("chat:")
                    || route.starts_with("settings:")
            };
            if state.route.as_deref() != Some(content_route.as_str()) {
                state.animate = state.route.as_ref().is_some_and(|previous| {
                    !route_without_transition(previous) && !route_without_transition(&content_route)
                });
                if state.animate {
                    state.generation = state.generation.wrapping_add(1);
                }
                state.route = Some(content_route);
            }
            state.animate.then_some(state.generation)
        });
        let outline_session = self.selected_session.and_then(|id| {
            self.store
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.sessions)
                .find(|session| session.id == id)
        });
        self.outline.refresh(outline_session);
        self.sync_chat_rows();
        self.sync_timeline(cx);
        if self.store.ui.performance_monitor {
            let timeline_visible = message_count > 0
                && !self.settings_ui.open
                && self.usage_ui.active.is_none()
                && self.github_ui.active.is_none()
                && self.active_diff.is_none()
                && self.active_file.is_none();
            self.performance.update(cx, |monitor, _| {
                monitor.begin_render(
                    message_count,
                    self.session_control.runs.len(),
                    self.store.workspaces.iter().map(|w| w.sessions.len()).sum(),
                );
                if !timeline_visible {
                    monitor.begin_message_frame();
                }
            });
        }
        let sidebar = self
            .sidebar_view
            .clone()
            .cached(StyleRefinement::default().size_full());
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let sidebar = chat::profiling::host_element(
            sidebar,
            ["sidebar_layout", "sidebar_prepaint", "sidebar_paint"],
        );
        let viewport_width = f32::from(window.viewport_size().width);
        let sidebar_state =
            window.use_keyed_state("body-resizable", cx, |_, _| ResizableState::default());
        let measured_sidebar = sidebar_state
            .read(cx)
            .sizes()
            .first()
            .map(|width| f32::from(*width))
            .filter(|width| *width > 0.)
            .unwrap_or(NAVIGATION_DEFAULT_WIDTH);
        let measured_sidebar = if self.plugins.collapsed() {
            38.
        } else {
            measured_sidebar
        };
        self.sidebar_width = measured_sidebar;
        let detail_state =
            window.use_keyed_state("detail-resizable", cx, |_, _| ResizableState::default());
        let content_width = (viewport_width - measured_sidebar - 38.).max(320.);
        let max_detail_width = (content_width * 0.5).max(260.);
        let measured_detail = detail_state
            .read(cx)
            .sizes()
            .get(1)
            .map(|width| f32::from(*width))
            .filter(|width| *width > 0.)
            .unwrap_or(self.plugins.dock.width);
        self.plugins.dock.width = measured_detail.clamp(260., max_detail_width);
        let detail_width = self.plugins.dock.width;
        let settings_width = (viewport_width
            - measured_sidebar
            - 38.
            - if viewport_width >= 1100. && self.right_view_visible() {
                detail_width
            } else {
                0.
            })
        .max(0.);
        self.update_content_pane_layout(settings_width, window, cx);
        let mut conversation = if self.settings_ui.open {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(self.render_settings_workspace(window, settings_width, cx))
        } else if let Some(panel) = self
            .github_ui
            .active
            .and_then(|id| self.github_ui.panels.get(&id))
            .cloned()
        {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(panel)
        } else if let Some(panel) = self
            .usage_ui
            .active
            .and_then(|id| self.usage_ui.panels.get(&id))
            .cloned()
        {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(panel)
        } else if self.active_diff.is_some() {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(self.render_diff_document(cx))
        } else if self.active_file.is_some() {
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(
                    self.file_view
                        .clone()
                        .cached(StyleRefinement::default().w_full().flex_1().min_h_0()),
                )
        } else if self.viewing_subagent() {
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .child(self.session_tab_bar(cx))
                .child(self.render_subagent_page(window, cx))
        } else {
            let messages = self.render_chat_messages(
                selected.as_ref(),
                self.timeline.clone(),
                TimelineSource::Main,
                self.chat_focus.clone(),
                cx,
            );
            let mut conversation = v_flex().flex_1().min_w_0().h_full();
            conversation = conversation.child(self.session_tab_bar(cx));
            conversation = conversation.child(messages);
            if let Some(id) = self.selected_session {
                conversation = conversation.children(self.render_chat_approval(id, window, cx));
            }
            let composer = {
                #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
                let _span = chat::profiling::host_span("composer_build");
                self.render_composer(selected.as_ref(), window, cx)
            };
            #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
            let composer = chat::profiling::host_element(
                composer,
                ["composer_layout", "composer_prepaint", "composer_paint"],
            );
            conversation = conversation.child(composer);
            conversation
        };
        conversation = conversation.child(self.notification_banner(cx));
        let toolbar = self.render_toolbar(window, cx);
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let toolbar = chat::profiling::host_element(
            toolbar,
            ["toolbar_layout", "toolbar_prepaint", "toolbar_paint"],
        );
        let title_bar = TitleBar::new()
            .bg(chrome_background(palette().canvas))
            .border_color(rgb(palette().border))
            .child(
                h_flex().items_center().gap_2().child(app_icon(20.)).child(
                    div()
                        .text_size(ui_text_size(13.))
                        .font_semibold()
                        .text_color(rgb(palette().text))
                        .child("Conduit"),
                ),
            );
        let dark_mode = gpui_kit::component::Theme::global(cx).is_dark();
        let theme_toggle_label = conduit::i18n::text_static(if dark_mode {
            "shell.switch_to_light_theme"
        } else {
            "shell.switch_to_dark_theme"
        });
        let title_bar = title_bar.child(div().flex_1()).child(
            h_flex()
                .id("title-bar-actions")
                .debug_selector(|| "title-bar-actions".into())
                .items_center()
                .gap_1()
                .mr_1()
                .flex_shrink_0()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(self.update_indicator(cx))
                .child(
                    Button::new("title-theme-toggle")
                        .debug_selector(|| "title-theme-toggle".into())
                        .accessibility_label(theme_toggle_label)
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(28.))
                        .flex_shrink_0()
                        .icon(if dark_mode {
                            FullIconName::Sun
                        } else {
                            FullIconName::Moon
                        })
                        .tooltip(theme_toggle_label)
                        .disabled(!self.writable || self.closing)
                        .when(self.writable && !self.closing, |button| {
                            button.cursor_pointer()
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.writable || this.closing {
                                return;
                            }
                            this.store.ui.appearance =
                                if gpui_kit::component::Theme::global(cx).is_dark() {
                                    AppearanceMode::Light
                                } else {
                                    AppearanceMode::Dark
                                };
                            this.persist();
                            this.apply_appearance(window, cx);
                        })),
                )
                .child(
                    Button::new("title-settings")
                        .debug_selector(|| "title-settings".into())
                        .accessibility_label(conduit::i18n::text_static("files.settings"))
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(28.))
                        .flex_shrink_0()
                        .icon(IconName::Settings)
                        .tooltip(conduit::i18n::format(
                            "shell.settings",
                            &[(
                                "arg0",
                                shortcuts::display("open_settings", &self.store.ui.shortcuts)
                                    .to_string(),
                            )],
                        ))
                        .cursor_pointer()
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                        ),
                )
                .child(
                    Button::new("title-about")
                        .debug_selector(|| "title-about".into())
                        .accessibility_label(conduit::i18n::text_static("settings.about"))
                        .ghost()
                        .small()
                        .compact()
                        .w(px(36.))
                        .h(px(28.))
                        .flex_shrink_0()
                        .icon(IconName::Info)
                        .tooltip(conduit::i18n::text_static("shell.about_conduit"))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_about_dialog(window, cx);
                        })),
                ),
        );
        let conversation = self.render_content_panes(conversation.into_any_element(), cx);
        let conversation = if self.settings_ui.open
            || self.usage_ui.active.is_some()
            || self.github_ui.active.is_some()
        {
            conversation
        } else {
            self.with_terminal_panel(conversation, window, cx)
        };
        let conversation = if self.branch_navigation_open()
            && !self.settings_ui.open
            && self.usage_ui.active.is_none()
            && self.github_ui.active.is_none()
        {
            let state =
                window.use_keyed_state("branch-split", cx, |_, _| ResizableState::default());
            let height = (f32::from(window.viewport_size().height) / 3.).clamp(180., 800.);
            let panel = h_flex()
                .id("branch-bottom-panel")
                .debug_selector(|| "branch-bottom-panel".into())
                .size_full()
                .min_h_0()
                .overflow_hidden()
                .bg(rgb(palette().canvas))
                .border_t_1()
                .border_color(rgb(palette().border))
                .children(self.branch_manager.clone());
            v_resizable("branch-split-group")
                .with_state(&state)
                .child(resizable_panel().child(conversation))
                .child(
                    resizable_panel()
                        .size(px(height))
                        .size_range(px(180.)..px(800.))
                        .child(panel),
                )
                .into_any_element()
        } else {
            conversation
        };
        let content = div().flex_1().min_h_0().child(conversation);
        let content = if let Some(content_motion_generation) = content_motion_generation {
            content
                .with_animation(
                    SharedString::from(format!(
                        "main-content-transition-{content_motion_generation}"
                    )),
                    Animation::new(Duration::from_millis(180)),
                    |element, progress| element.opacity(0.78 + progress * 0.22),
                )
                .into_any_element()
        } else {
            content.into_any_element()
        };
        let conversation = v_flex().flex_1().min_w_0().h_full().child(content).when(
            !self.settings_ui.open
                && self.usage_ui.active.is_none()
                && self.github_ui.active.is_none(),
            |layout| layout.child(toolbar),
        );
        // Both sidebars use the same resizable panels and overlay handles.
        // A nested group keeps the right sidebar independent of left navigation.
        let (detail_panel, overlay) = self.render_detail_dock(window, cx);
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let detail_panel = detail_panel.map(|panel| {
            chat::profiling::host_element(panel, ["dock_layout", "dock_prepaint", "dock_paint"])
        });

        let body = h_resizable("body").with_state(&sidebar_state).child(
            resizable_panel()
                .size(px(NAVIGATION_DEFAULT_WIDTH))
                .size_range(px(NAVIGATION_MIN_WIDTH)..px(NAVIGATION_MAX_WIDTH))
                .fixed()
                .child(sidebar),
        );
        let conversation_area: AnyElement = match detail_panel {
            Some(panel) => h_resizable("detail-split")
                .with_state(&detail_state)
                .child(resizable_panel().child(conversation))
                .child(
                    resizable_panel()
                        .size(px(detail_width))
                        .size_range(px(260.)..px(max_detail_width))
                        .fixed()
                        .child(panel),
                )
                .into_any_element(),
            None => conversation.into_any_element(),
        };
        let body = if self.plugins.collapsed() {
            h_flex()
                .size_full()
                .child(
                    div().w(px(38.)).h_full().flex_shrink_0().child(
                        self.sidebar_view
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
                )
                .child(conversation_area)
                .into_any_element()
        } else {
            body.child(resizable_panel().child(conversation_area))
                .into_any_element()
        };

        let mut main_area = div().relative().flex_1().min_h_0().child(
            h_flex()
                .size_full()
                .child(div().flex_1().min_w_0().h_full().child(body))
                .child(self.render_right_navigation(cx)),
        );
        if let Some(panel) = overlay {
            main_area = main_area.child(panel);
        }
        let view = v_flex()
            .relative()
            .size_full()
            .bg(rgb(palette().canvas))
            .text_color(rgb(palette().text))
            .when(theme::background_visible(), |view| {
                view.child(
                    div().absolute().inset_0().child(
                        self.background
                            .layer
                            .clone()
                            .cached(StyleRefinement::default().size_full()),
                    ),
                )
            })
            .child(title_bar)
            .child(main_area);
        if self.store.ui.performance_monitor {
            self.performance.update(cx, |monitor, _| {
                monitor.record_render(render_started.elapsed());
            });
        }
        let view = view
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                if event.keystroke.key != "shift" {
                    this.double_shift.cancel();
                }
            }))
            .capture_any_mouse_down(cx.listener(|this, _, _, _| {
                this.double_shift.cancel();
            }))
            .on_modifiers_changed(
                cx.listener(|this, event: &ModifiersChangedEvent, window, cx| {
                    if shortcuts::get("search_files")
                        .keys(&this.store.ui.shortcuts)
                        .iter()
                        .any(|key| key == shortcuts::DOUBLE_SHIFT)
                        && !this.terminal_focused(window, cx)
                        && !window.has_active_dialog(cx)
                        && this.double_shift.update(event.modifiers, Instant::now())
                    {
                        this.open_file_search(window, cx);
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                this.terminal_command(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.terminal_command(true, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &LeaveTerminal, window, cx| this.leave_terminal(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextTerminal, window, cx| {
                this.cycle_terminal(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PreviousTerminal, window, cx| {
                this.cycle_terminal(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &OpenFileSearchShortcut, window, cx| {
                this.open_file_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RecentSessions, window, cx| {
                this.open_recent_sessions(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSessionSearch, window, cx| {
                this.open_search_dialog(window, cx);
            }))
            .on_action(cx.listener(|this, _: &OpenSettingsShortcut, window, cx| {
                this.open_settings(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleGitNavigation, _, cx| {
                this.toggle_git_navigation(cx);
            }))
            .into_any_element();
        #[cfg(all(test, feature = "ui-tests", conduit_shell_desktop))]
        let view =
            chat::profiling::host_element(view, ["host_layout", "host_prepaint", "host_paint"]);
        view
    }
}
