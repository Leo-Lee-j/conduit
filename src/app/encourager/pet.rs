use super::*;
use pet_core::{Animator, Config, PetState, StateMachine};
mod host;
mod window;
pub(in crate::app) use host::PetFeedback;
pub(super) use host::PetOutput;
use window::*;
pub(super) use window::{pet_window_bounds, position_from_window};

pub(crate) struct PetView {
    app: WeakEntity<ConduitApp>,
    output: PetOutput,
    focus: FocusHandle,
    machine: StateMachine,
    animator: Animator,
    thinking_until: Option<Instant>,
    feedback_pending: bool,
    feedback_visible: bool,
    feedback_until: Option<Instant>,
    feedback_remaining: Option<Duration>,
    menu: bool,
    bubble_hovered: bool,
    scale: f32,
    visible_region: std::rc::Rc<std::cell::RefCell<Vec<[i32; 4]>>>,
    press: Option<PetPress>,
    position_due: Option<Instant>,
    persisted_position: crate::store::EncouragerPetPosition,
    wake: Option<Instant>,
    timer: Option<Task<()>>,
    drag_watch: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}
impl PetView {
    pub(super) fn new(
        app: Entity<ConduitApp>,
        output: PetOutput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let now = cx.background_executor().now();
        let output_subscription = cx.observe(&app, |this, app, cx| {
            let output = PetOutput::from_app(app.read(cx));
            if this.output != output {
                this.accept_output(output, cx.background_executor().now());
                cx.notify();
            }
        });
        let completion_subscription = cx.subscribe(&app, |this, _, event: &PetFeedback, cx| {
            this.machine
                .feedback(event.failed, cx.background_executor().now());
            cx.notify();
        });
        let bounds_subscription = cx.observe_window_bounds(window, |this, window, cx| {
            if this.scale != window.scale_factor() {
                cx.notify();
            }
            if position_from_window(window) != this.persisted_position {
                this.position_due = Some(cx.background_executor().now() + Duration::from_secs(1));
                cx.notify();
            }
        });
        let parent = app.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let position = position_from_window(window);
            let _ = parent.update(cx, |app, cx| {
                app.encourager_pet_window_closed(position);
                cx.notify();
            });
            true
        });
        let mut machine = StateMachine::new(Config::default(), now);
        machine.sync_activity(
            output.loading || !output.runs.is_empty(),
            output.runs.iter().any(|(_, _, attention)| *attention),
            now,
        );
        let thinking_until = output.loading.then_some(now + Duration::from_millis(600));
        Self {
            app: app.downgrade(),
            output,
            focus: cx.focus_handle(),
            machine,
            animator: Animator::new(0x706978656c706574, now),
            thinking_until,
            feedback_pending: false,
            feedback_visible: false,
            feedback_until: None,
            feedback_remaining: None,
            menu: false,
            bubble_hovered: false,
            scale: 0.,
            visible_region: Default::default(),
            press: None,
            position_due: None,
            persisted_position: position_from_window(window),
            wake: None,
            timer: None,
            drag_watch: None,
            _subscriptions: vec![
                output_subscription,
                completion_subscription,
                bounds_subscription,
            ],
        }
    }
    fn accept_output(&mut self, output: PetOutput, now: Instant) {
        if output.runs != self.output.runs {
            self.machine.activity(now);
        }
        if output.runs.iter().any(|(id, started, _)| {
            !self
                .output
                .runs
                .iter()
                .any(|(old_id, old_started, _)| id == old_id && started == old_started)
        }) || (output.loading && !self.output.loading)
        {
            self.machine.event(pet_core::PetEvent::TaskStarted, now);
        }
        if output.loading && !self.output.loading {
            self.thinking_until = Some(now + Duration::from_millis(600));
            self.feedback_visible = false;
            self.feedback_until = None;
            self.feedback_remaining = None;
            self.feedback_pending = false;
        } else if !output.loading
            && (output.reply.is_some() || output.error.is_some())
            && (self.output.loading
                || output.reply != self.output.reply
                || output.error != self.output.error)
        {
            self.feedback_pending = true;
        } else if !output.loading && output.reply.is_none() && output.error.is_none() {
            self.feedback_pending = false;
            self.feedback_visible = false;
            self.feedback_until = None;
            self.feedback_remaining = None;
        }
        self.machine.sync_activity(
            output.loading || !output.runs.is_empty(),
            output.runs.iter().any(|(_, _, attention)| *attention),
            now,
        );
        self.output = output;
    }
    fn advance(&mut self, now: Instant, window: &mut Window, cx: &mut Context<Self>) {
        if self.thinking_until.is_some_and(|until| until <= now) {
            self.thinking_until = None;
        }
        if self.feedback_pending && self.thinking_until.is_none() && !self.output.loading {
            self.feedback_pending = false;
            self.feedback_visible = true;
            self.feedback_until = Some(now + Duration::from_secs(25));
            self.feedback_remaining = None;
            self.machine.feedback(self.output.error.is_some(), now);
            self.pause_feedback(self.menu || self.bubble_hovered, now);
        }
        if self.feedback_until.is_some_and(|until| until <= now) {
            self.feedback_visible = false;
            self.feedback_until = None;
            self.feedback_remaining = None;
        }
        if self.position_due.is_some_and(|until| until <= now) {
            self.save_position(window, cx);
        }
        self.sync_size(window, cx);
    }
    fn schedule(&mut self, wake: Option<Instant>, window: &Window, cx: &mut Context<Self>) {
        if self.wake == wake {
            return;
        }
        self.wake = wake;
        self.timer = wake.map(|at| {
            let timer = cx
                .background_executor()
                .timer(at.saturating_duration_since(cx.background_executor().now()));
            cx.spawn_in(window, async move |this, cx| {
                timer.await;
                let _ = this.update_in(cx, |this, _, cx| {
                    this.wake = None;
                    cx.notify();
                });
            })
        });
    }
    fn pause_feedback(&mut self, pause: bool, now: Instant) {
        if pause {
            if let Some(until) = self.feedback_until.take() {
                self.feedback_remaining = Some(until.saturating_duration_since(now));
            }
        } else if let Some(remaining) = self.feedback_remaining.take() {
            self.feedback_until = Some(now + remaining);
        }
    }
    fn sync_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scale = window.scale_factor();
        if scale != self.scale {
            self.scale = scale;
            // Bounds observers update PetView synchronously. Run the native
            // mutation after releasing this entity's lease, including render.
            window.defer(cx, resize_window);
            cx.notify();
        }
    }
    fn save_position(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.position_due = None;
        let position = position_from_window(window);
        if self.persisted_position == position {
            return;
        }
        self.persisted_position = position;
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, _| {
                if app.store.ui.encourager.pet_position != Some(position) {
                    app.store.ui.encourager.pet_position = Some(position);
                    app.persist();
                }
            });
        }
    }
    fn interact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.thinking_until.is_some() || self.output.loading || !self.output.enabled {
            return;
        }
        let now = cx.background_executor().now();
        self.menu = false;
        self.feedback_visible = false;
        self.feedback_pending = false;
        self.feedback_until = None;
        self.feedback_remaining = None;
        self.bubble_hovered = false;
        self.machine.poke(now);
        self.thinking_until = Some(now + Duration::from_millis(600));
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| app.generate_encouragement(cx));
            let output = PetOutput::from_app(app.read(cx));
            self.accept_output(output, now);
            self.feedback_pending = !self.output.loading
                && (self.output.reply.is_some() || self.output.error.is_some());
        }
        self.sync_size(window, cx);
        cx.notify();
    }
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = false;
        self.feedback_visible = false;
        self.feedback_until = None;
        self.feedback_remaining = None;
        self.bubble_hovered = false;
        self.machine.activity(cx.background_executor().now());
        self.sync_size(window, cx);
        cx.notify();
    }
    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = position_from_window(window);
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| {
                app.encourager_pet_window_closed(position);
                cx.notify();
            });
        }
        window.remove_window();
    }
    fn finish_press(&mut self, click: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.press.take() else {
            return;
        };
        self.drag_watch = None;
        self.machine.drag(false, cx.background_executor().now());
        if press.dragged {
            self.position_due = Some(cx.background_executor().now() + Duration::from_secs(1));
        } else if click {
            self.interact(window, cx);
        }
        cx.notify();
    }
    fn move_press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(press) = self.press.filter(|press| press.dragged) else {
            return;
        };
        self.machine.drag(true, cx.background_executor().now());
        #[cfg(target_os = "windows")]
        if let Some(native) = press.native {
            window.defer(cx, move |window, cx| native_drag(window, native, cx));
        }
        #[cfg(not(target_os = "windows"))]
        window.start_window_move();
        cx.notify();
    }
    fn start_drag_watch(&mut self, window: &Window, cx: &mut Context<Self>) {
        // A background no-activate window cannot rely on capture outside its bounds.
        // This guard exists ONLY during a physical press and never while idle.
        #[cfg(target_os = "windows")]
        if self.press.is_some_and(|press| press.native.is_some()) {
            self.drag_watch = Some(cx.spawn_in(window, async move |this, cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(16))
                        .await;
                    let keep = this
                        .update_in(cx, |this, window, cx| {
                            let Some(press) = this.press else {
                                return false;
                            };
                            match native_motion(press) {
                                Some((true, moved)) => {
                                    if moved {
                                        this.press.as_mut().unwrap().dragged = true;
                                    }
                                    this.move_press(window, cx);
                                    true
                                }
                                _ => {
                                    this.finish_press(false, window, cx);
                                    false
                                }
                            }
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            }));
        }
        #[cfg(not(target_os = "windows"))]
        let _ = (window, cx);
    }
}
fn state_name(state: PetState) -> &'static str {
    match state {
        PetState::Idle => "idle",
        PetState::Working => "working",
        PetState::Attention => "attention",
        PetState::Success => "success",
        PetState::Error => "error",
        PetState::Sleep => "sleep",
        PetState::Poked => "poked",
        PetState::Dragged => "dragged",
    }
}

impl Render for PetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = cx.background_executor().now();
        self.advance(now, window, cx);
        let (state, state_wake) = self.machine.tick(now);
        self.animator.set_state(state, now);
        let tick = self.animator.tick(now);
        let frame = pet_core::rasterize(&tick.pose, &pet_core::Palette::default());
        self.schedule(
            [
                state_wake,
                tick.next_wake,
                self.thinking_until,
                self.feedback_until,
                self.position_due,
            ]
            .into_iter()
            .flatten()
            .min(),
            window,
            cx,
        );
        let scale = window.scale_factor();
        let cell = cell_size(scale);
        let compact = compact_size(scale);
        let thinking = self.thinking_until.is_some() || self.output.loading;
        let art = div()
            .id("desktop-pet-art")
            .debug_selector(move || {
                if thinking {
                    "desktop-pet-thinking".into()
                } else {
                    format!("desktop-pet-{}", state_name(state))
                }
            })
            .w(compact.width)
            .h(compact.height)
            .child(
                gpui_kit::canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let origin = point(
                            px((f32::from(bounds.origin.x) * scale).round() / scale),
                            px((f32::from(bounds.origin.y) * scale).round() / scale),
                        );
                        for r in &frame.rects {
                            window.paint_quad(gpui_kit::fill(
                                Bounds::new(
                                    origin + point(cell * r.x as f32, cell * r.y as f32),
                                    size(cell * r.w as f32, cell * r.h as f32),
                                ),
                                gpui_kit::rgba(r.color),
                            ));
                        }
                    },
                )
                .size_full(),
            );
        let accessible_pet = cx.weak_entity();
        let pet = div()
            .id("desktop-pet-character")
            .debug_selector(|| "desktop-pet-character".into())
            .absolute()
            .right_0()
            .bottom_0()
            .w(compact.width)
            .h(compact.height)
            .cursor_pointer()
            .role(gpui_kit::accesskit::Role::Button)
            .aria_label(conduit::i18n::text_static(if thinking {
                "encourager.pet.thinking"
            } else {
                "encourager.pet.interact"
            }))
            .aria_description(conduit::i18n::text_static(
                "encourager.pet.setting_description",
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    let now = cx.background_executor().now();
                    this.machine.activity(now);
                    this.press = Some(PetPress::new(event.position, window));
                    this.start_drag_watch(window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button != Some(MouseButton::Left) {
                    this.finish_press(false, window, cx);
                    return;
                }
                let Some(press) = this.press.as_mut() else {
                    return;
                };
                let delta = event.position - press.local;
                if f32::from(delta.x).abs().max(f32::from(delta.y).abs()) * window.scale_factor()
                    >= 4.
                {
                    press.dragged = true;
                }
                this.move_press(window, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish_press(true, window, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.finish_press(false, window, cx)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| {
                    this.menu = !this.menu;
                    this.machine.activity(cx.background_executor().now());
                    this.pause_feedback(
                        this.menu || this.bubble_hovered,
                        cx.background_executor().now(),
                    );
                    this.sync_size(window, cx);
                    cx.notify();
                }),
            )
            .on_a11y_action(gpui_kit::accesskit::Action::Click, move |_, window, cx| {
                let _ = accessible_pet.update(cx, |this, cx| this.interact(window, cx));
            })
            .child(art);
        let visible_region_cache = self.visible_region.clone();
        let mut content = div()
            .on_children_prepainted(move |bounds, window, cx| {
                let region = visible_region(&bounds, window.scale_factor());
                if *visible_region_cache.borrow() != region {
                    *visible_region_cache.borrow_mut() = region.clone();
                    window.defer(cx, move |window, _| set_visible_region(window, &region));
                }
            })
            .id("desktop-pet")
            .debug_selector(|| "desktop-pet".into())
            .relative()
            .size_full()
            .track_focus(&self.focus)
            .font_family(cx.theme().font_family.clone())
            .text_color(rgb(palette().text))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => this.dismiss(window, cx),
                    "enter" | "space" => this.interact(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(pet);
        if self.feedback_visible || self.menu {
            let mut bubble = v_flex()
                .id("desktop-pet-bubble")
                .debug_selector(|| "desktop-pet-bubble".into())
                .absolute()
                .right(px(8.))
                .bottom(compact.height)
                .w(px(WITH_BUBBLE.0 - 16.))
                .max_h(px(WITH_BUBBLE.1 - 8.) - compact.height)
                .when(!self.menu, |this| {
                    this.h(px(WITH_BUBBLE.1 - 8.) - compact.height)
                })
                .p(px(12.))
                .gap(px(8.))
                .rounded_lg()
                .bg(rgb(palette().surface_strong))
                .on_hover(cx.listener(|this, hovered, _, cx| {
                    this.bubble_hovered = *hovered;
                    let now = cx.background_executor().now();
                    this.machine.activity(now);
                    this.pause_feedback(*hovered || this.menu, now);
                    cx.notify();
                }));
            if !self.menu {
                bubble = bubble.child(
                    div()
                        .id("desktop-pet-feedback")
                        .debug_selector(|| "desktop-pet-feedback".into())
                        .w_full()
                        .min_w_0()
                        .min_h_0()
                        .flex_1()
                        .overflow_y_scroll()
                        .text_size(px(13.))
                        .line_height(relative(1.5))
                        .when(self.output.error.is_some(), |this| {
                            this.text_color(rgb(palette().danger_text))
                        })
                        .child(
                            self.output
                                .error
                                .clone()
                                .or_else(|| self.output.reply.clone())
                                .unwrap_or_default(),
                        ),
                );
            }
            let mut actions = h_flex()
                .flex_shrink_0()
                .gap_2()
                .flex_wrap()
                .child(
                    pet_action("desktop-pet-again", "encourager.again")
                        .on_click(cx.listener(|this, _, window, cx| this.interact(window, cx))),
                )
                .child(
                    pet_action("desktop-pet-dismiss", "encourager.pet.dismiss")
                        .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
                );
            if self.menu {
                actions = actions.child(
                    pet_action("desktop-pet-hide", "encourager.pet.hide")
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                );
            } else if self.output.reply.is_some() && self.output.error.is_none() {
                actions = actions.child(
                    pet_action("desktop-pet-copy", "encourager.pet.copy").on_click(cx.listener(
                        |this, _, _, cx| {
                            if let Some(reply) = &this.output.reply {
                                cx.write_to_clipboard(ClipboardItem::new_string(reply.clone()));
                            }
                        },
                    )),
                );
            }
            bubble = bubble.child(actions);
            content = content.child(bubble);
        }
        content
    }
}

fn pet_action(id: &'static str, label: &'static str) -> gpui_kit::Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .px_2()
        .py_1()
        .rounded_md()
        .text_size(px(12.))
        .cursor_pointer()
        .hover(|this| this.bg(rgb(palette().surface_hover)))
        .role(gpui_kit::accesskit::Role::Button)
        .aria_label(conduit::i18n::text_static(label))
        .child(conduit::i18n::text_static(label))
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests;
