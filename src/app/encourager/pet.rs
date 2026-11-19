#[cfg(target_os = "windows")]
use super::native_window::handle as native_handle;
use super::*;

const COMPACT: (f32, f32) = (148., 112.);
const WITH_BUBBLE: (f32, f32) = (320., 312.);
const TICK: Duration = Duration::from_millis(180);
// The periodic timer may tick immediately after a click. Four ticks guarantee
// at least three full intervals (~540 ms) of visible thinking.
const MIN_THINKING_TICKS: u32 = 4;
const FEEDBACK_TICKS: u32 = 140;
const EDGE_INSET: f32 = 16.;

pub(crate) struct PetView {
    app: WeakEntity<ConduitApp>,
    output: PetOutput,
    focus: FocusHandle,
    thinking_ticks: Option<u32>,
    feedback_ticks: Option<u32>,
    menu: bool,
    bubble_hovered: bool,
    expanded: bool,
    press: Option<PetPress>,
    pending_position: crate::store::EncouragerPetPosition,
    persisted_position: crate::store::EncouragerPetPosition,
    position_changed_at: Instant,
    _subscription: Subscription,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct PetOutput {
    reply: Option<String>,
    error: Option<String>,
    loading: bool,
    enabled: bool,
}

impl PetOutput {
    pub(super) fn from_app(app: &ConduitApp) -> Self {
        Self {
            reply: app.encourager.reply.clone(),
            error: app.encourager.error.clone(),
            loading: app.encourager.loading,
            enabled: app.store.ui.encourager.enabled && app.writable && !app.closing,
        }
    }
}

#[derive(Clone, Copy)]
struct PetPress {
    local: Point<Pixels>,
    dragged: bool,
    #[cfg(target_os = "windows")]
    native: Option<(i32, i32, i32, i32)>,
}

impl PetView {
    pub(super) fn new(
        app: Entity<ConduitApp>,
        output: PetOutput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.observe(&app, |this, app, cx| {
            let output = PetOutput::from_app(app.read(cx));
            if this.output == output {
                return;
            }
            if output.loading && !this.output.loading {
                this.thinking_ticks = Some(0);
                this.feedback_ticks = None;
            } else if this.thinking_ticks.is_none()
                && (output.reply.is_some() || output.error.is_some())
            {
                this.feedback_ticks = Some(0);
            }
            this.output = output;
            cx.notify();
        });
        let parent = app.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let position = position_from_bounds(window.bounds());
            let _ = parent.update(cx, |app, cx| {
                app.encourager_pet_window_closed(position);
                cx.notify();
            });
            true
        });
        let initial_position = position_from_bounds(window.bounds());
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                if this
                    .update_in(cx, |pet, window, cx| pet.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let thinking_ticks = output.loading.then_some(0);
        Self {
            app: app.downgrade(),
            output,
            focus: cx.focus_handle(),
            thinking_ticks,
            feedback_ticks: None,
            menu: false,
            bubble_hovered: false,
            expanded: false,
            press: None,
            pending_position: initial_position,
            persisted_position: initial_position,
            position_changed_at: Instant::now(),
            _subscription: subscription,
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ticks) = &mut self.thinking_ticks {
            *ticks = ticks.saturating_add(1);
            if *ticks >= MIN_THINKING_TICKS && !self.output.loading {
                self.thinking_ticks = None;
                self.feedback_ticks =
                    (self.output.reply.is_some() || self.output.error.is_some()).then_some(0);
            }
            cx.notify();
        } else if let Some(ticks) = &mut self.feedback_ticks
            && !self.bubble_hovered
            && !self.menu
        {
            *ticks += 1;
            if *ticks >= FEEDBACK_TICKS {
                self.feedback_ticks = None;
                cx.notify();
            }
        }
        self.sync_size(window, cx);
        let position = position_from_bounds(window.bounds());
        if self.pending_position != position {
            self.pending_position = position;
            self.position_changed_at = Instant::now();
        }
        if self.pending_position != self.persisted_position
            && self.position_changed_at.elapsed() >= Duration::from_millis(900)
        {
            self.persisted_position = self.pending_position;
            if let Some(app) = self.app.upgrade() {
                app.update(cx, |app, _| {
                    if app.store.ui.encourager.pet_position != Some(position) {
                        app.store.ui.encourager.pet_position = Some(position);
                        app.persist();
                    }
                });
            }
        }
    }

    fn sync_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let expanded = self.feedback_ticks.is_some() || self.menu;
        if expanded == self.expanded {
            return;
        }
        self.expanded = expanded;
        resize_window(window, expanded, cx);
        cx.notify();
    }

    fn interact(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.thinking_ticks.is_some() || self.output.loading || !self.output.enabled {
            return;
        }
        self.menu = false;
        self.feedback_ticks = None;
        self.thinking_ticks = Some(0);
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| app.generate_encouragement(cx));
            // Validation can finish synchronously. Still show the thinking pose
            // before presenting its feedback, and never issue a second request.
            self.output = PetOutput::from_app(app.read(cx));
        }
        self.sync_size(window, cx);
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = false;
        self.feedback_ticks = None;
        self.bubble_hovered = false;
        self.sync_size(window, cx);
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let position = position_from_bounds(window.bounds());
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| {
                app.encourager_pet_window_closed(position);
                cx.notify();
            });
        }
        window.remove_window();
    }

    fn character(&self) -> [&'static str; 4] {
        if let Some(frame) = self.thinking_ticks {
            match (frame / 2) % 3 {
                0 => ["  .      ", " /\\_/\\  ", "( -.- )  ", " / ? \\  "],
                1 => ["  ..     ", " /\\_/\\  ", "( o.- )  ", " / ? \\  "],
                _ => ["  ...    ", " /\\_/\\  ", "( -.o )  ", " / ? \\  "],
            }
        } else if self.feedback_ticks.is_some() && self.output.error.is_some() {
            ["         ", " /\\_/\\  ", "( o.o )? ", " /   \\  "]
        } else if self.feedback_ticks.is_some() {
            ["         ", " /\\_/\\  ", "( ^.^ )  ", " / > <\\ "]
        } else {
            ["         ", " /\\_/\\  ", "( o.o )  ", " / > <\\ "]
        }
    }
}

impl Render for PetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let character = self.character();
        let thinking = self.thinking_ticks.is_some();
        let mut art = div()
            .id("desktop-pet-art")
            .debug_selector(move || {
                if thinking {
                    "desktop-pet-thinking"
                } else {
                    "desktop-pet-idle"
                }
                .into()
            })
            .relative()
            .w(px(132.))
            .h(px(104.));
        // Outline the glyphs rather than painting a rectangular backing surface.
        // Keep every frame on the same monospace grid, independent of UI font size.
        for (x, y, color) in [
            (-1., 0., 0x292521),
            (1., 0., 0x292521),
            (0., -1., 0x292521),
            (0., 1., 0x292521),
            (0., 0., 0xffe3ae),
        ] {
            art = art.child(
                v_flex()
                    .absolute()
                    .left(px(8. + x))
                    .top(px(4. + y))
                    .w(px(124.))
                    .font_family("Consolas")
                    .text_size(px(22.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(color))
                    .children(character.into_iter().map(|line| {
                        div()
                            .h(px(24.))
                            .line_height(px(24.))
                            .whitespace_nowrap()
                            .child(line)
                    })),
            );
        }
        let accessible_pet = cx.entity();
        let pet = div()
            .id("desktop-pet-character")
            .debug_selector(|| "desktop-pet-character".into())
            .absolute()
            .right_0()
            .bottom_0()
            .w(px(COMPACT.0))
            .h(px(COMPACT.1))
            .cursor_pointer()
            .role(gpui_kit::accesskit::Role::Button)
            .aria_label(conduit::i18n::text_static(
                if self.thinking_ticks.is_some() {
                    "encourager.pet.thinking"
                } else {
                    "encourager.pet.interact"
                },
            ))
            .aria_description(conduit::i18n::text_static(
                "encourager.pet.setting_description",
            ))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.press = Some(PetPress {
                        local: event.position,
                        dragged: false,
                        #[cfg(target_os = "windows")]
                        native: native_press(window),
                    });
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                let Some(press) = &mut this.press else {
                    return;
                };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.press = None;
                    return;
                }
                let delta = event.position - press.local;
                if f32::from(delta.x).abs().max(f32::from(delta.y).abs()) >= 6. {
                    press.dragged = true;
                }
                if press.dragged {
                    #[cfg(target_os = "windows")]
                    if let Some(native) = press.native {
                        // Defer native movement until the input handler has released
                        // its entity lease; SetWindowPos may synchronously dispatch.
                        cx.defer_in(window, move |_, window, cx| native_drag(window, native, cx));
                    }
                    #[cfg(not(target_os = "windows"))]
                    window.start_window_move();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.press.take().is_some_and(|press| !press.dragged) {
                        this.interact(window, cx);
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.press = None),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| {
                    this.focus.focus(window, cx);
                    this.menu = !this.menu;
                    this.sync_size(window, cx);
                    cx.notify();
                }),
            )
            .on_a11y_action(gpui_kit::accesskit::Action::Click, move |_, window, cx| {
                accessible_pet.update(cx, |this, cx| this.interact(window, cx));
            })
            .child(art);
        let mut content = div()
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
        if self.feedback_ticks.is_some() || self.menu {
            let mut bubble = v_flex()
                .id("desktop-pet-bubble")
                .debug_selector(|| "desktop-pet-bubble".into())
                .absolute()
                .right(px(8.))
                .bottom(px(COMPACT.1))
                .w(px(WITH_BUBBLE.0 - 16.))
                .max_h(px(WITH_BUBBLE.1 - COMPACT.1 - 8.))
                .p(px(12.))
                .gap(px(8.))
                .rounded_lg()
                .bg(rgb(palette().surface_strong))
                .on_hover(cx.listener(|this, hovered, _, _| this.bubble_hovered = *hovered));
            if !self.menu {
                bubble = bubble.child(
                    div()
                        .id("desktop-pet-feedback")
                        .w_full()
                        .min_w_0()
                        .max_h(px(128.))
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

pub(super) fn position_from_bounds(bounds: Bounds<Pixels>) -> crate::store::EncouragerPetPosition {
    // Persist the compact character's origin, not the temporary bubble's origin.
    crate::store::EncouragerPetPosition {
        x: f32::from(bounds.origin.x + bounds.size.width - px(COMPACT.0)).round() as i32,
        y: f32::from(bounds.origin.y + bounds.size.height - px(COMPACT.1)).round() as i32,
    }
}

fn visible_bounds(anchor: Point<Pixels>, cx: &App) -> Option<Bounds<Pixels>> {
    cx.displays()
        .into_iter()
        .find(|display| display.visible_bounds().contains(&anchor))
        .or_else(|| cx.primary_display())
        .map(|display| display.visible_bounds())
}

pub(super) fn pet_window_bounds(
    saved: Option<crate::store::EncouragerPetPosition>,
    cx: &App,
) -> Bounds<Pixels> {
    let anchor = saved
        .map(|p| point(px(p.x as f32), px(p.y as f32)))
        .unwrap_or_default();
    let pet_size = size(px(COMPACT.0), px(COMPACT.1));
    let Some(visible) = visible_bounds(anchor, cx) else {
        return Bounds::centered(None, pet_size, cx);
    };
    let max = visible.origin + visible.size.into()
        - pet_size.into()
        - point(px(EDGE_INSET), px(EDGE_INSET));
    let min = visible.origin + point(px(EDGE_INSET), px(EDGE_INSET));
    let origin = if saved.is_some() {
        point(
            anchor.x.clamp(min.x, max.x.max(min.x)),
            anchor.y.clamp(min.y, max.y.max(min.y)),
        )
    } else {
        max
    };
    Bounds::new(origin, pet_size)
}

fn resize_window(window: &mut Window, expanded: bool, cx: &mut App) {
    let (width, height) = if expanded { WITH_BUBBLE } else { COMPACT };
    let next_size = size(px(width), px(height));
    let old = window.bounds();
    let mut origin = old.origin + old.size.into() - next_size.into();
    if let Some(visible) = visible_bounds(old.center(), cx) {
        let max = visible.origin + visible.size.into() - next_size.into();
        origin.x = origin
            .x
            .clamp(visible.origin.x, max.x.max(visible.origin.x));
        origin.y = origin
            .y
            .clamp(visible.origin.y, max.y.max(visible.origin.y));
    }
    #[cfg(target_os = "windows")]
    if let Some(hwnd) = native_handle(window) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos,
        };
        let scale = window.scale_factor();
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                (f32::from(origin.x) * scale).round() as i32,
                (f32::from(origin.y) * scale).round() as i32,
                (width * scale).round() as i32,
                (height * scale).round() as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
        window.bounds_changed(cx);
        return;
    }
    window.resize(next_size);
}

#[cfg(target_os = "windows")]
fn native_press(window: &Window) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect},
    };
    let hwnd = native_handle(window)?;
    let mut point = POINT::default();
    let mut rect = RECT::default();
    unsafe {
        if GetCursorPos(&mut point) == 0 || GetWindowRect(hwnd, &mut rect) == 0 {
            return None;
        }
    }
    Some((point.x, point.y, rect.left, rect.top))
}

#[cfg(target_os = "windows")]
fn native_drag(window: &mut Window, (x, y, left, top): (i32, i32, i32, i32), cx: &mut App) {
    use windows_sys::Win32::{
        Foundation::POINT,
        UI::WindowsAndMessaging::{
            GetCursorPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
        },
    };
    let Some(hwnd) = native_handle(window) else {
        return;
    };
    let mut point = POINT::default();
    unsafe {
        if GetCursorPos(&mut point) != 0 {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                left + point.x - x,
                top + point.y - y,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
            );
        }
    }
    window.bounds_changed(cx);
}

#[cfg(all(test, feature = "ui-tests"))]
mod tests {
    use gpui_kit::{AppContext as _, TestAppContext, point, px};

    fn pet_store(pet_enabled: bool) -> crate::store::Store {
        let mut store = crate::store::Store::default();
        store.ui.encourager.enabled = true;
        store.ui.encourager.pet_enabled = pet_enabled;
        store
    }

    #[gpui_kit::test]
    fn desktop_pet_is_transparent_and_clicks_think_before_feedback(cx: &mut TestAppContext) {
        use gpui_kit::{Modifiers, MouseButton, VisualTestContext};
        use std::time::Duration;
        let _locale = crate::test_support::chinese();
        cx.update(gpui_kit::init);
        let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
        let output = slot.clone();
        let (_, host) = cx.add_window_view(|window, cx| {
            let app = cx.new(|cx| {
                let mut app =
                    super::ConduitApp::with_store(window, cx, Ok(pet_store(false)), false, false);
                app.persistence = crate::persistence::Persistence::with_writer(|_| Ok(()));
                app.store.providers.clear();
                app
            });
            *output.borrow_mut() = Some(app.clone());
            app.update(cx, |app, cx| app.set_encourager_pet_enabled(true, cx));
            gpui_kit::component::Root::new(app, window, cx)
        });
        let app = slot.borrow().as_ref().unwrap().clone();
        host.run_until_parked();
        let handle = host.update(|_, cx| app.read(cx).encourager_pet_window.unwrap());
        let mut pet = VisualTestContext::from_window(handle, cx);
        pet.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(
                window.painted_quads().is_empty(),
                "idle pet must not paint any backing panel or Root background"
            );
        });
        assert!(pet.debug_bounds("desktop-pet-character").is_some());
        assert!(pet.debug_bounds("desktop-pet-bubble").is_none());
        let center = pet.debug_bounds("desktop-pet-character").unwrap().center();

        // A drag must not be interpreted as a chat request on mouse-up.
        let generation = pet.update(|_, cx| app.read(cx).encourager.generation);
        pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        pet.simulate_mouse_move(
            center + point(px(12.), px(0.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        pet.simulate_mouse_up(
            center + point(px(12.), px(0.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        pet.update(|_, cx| assert_eq!(app.read(cx).encourager.generation, generation));

        pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        pet.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        pet.run_until_parked();
        pet.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(pet.debug_bounds("desktop-pet-thinking").is_some());
        assert!(
            pet.debug_bounds("desktop-pet-bubble").is_none(),
            "even immediate validation feedback follows the thinking pose"
        );
        let generation = pet.update(|_, cx| app.read(cx).encourager.generation);
        pet.simulate_mouse_down(center, MouseButton::Left, Modifiers::default());
        pet.simulate_mouse_up(center, MouseButton::Left, Modifiers::default());
        pet.update(|_, cx| assert_eq!(app.read(cx).encourager.generation, generation));

        for _ in 0..4 {
            pet.executor().advance_clock(Duration::from_millis(180));
            pet.run_until_parked();
        }
        pet.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(pet.debug_bounds("desktop-pet-thinking").is_none());
        assert!(pet.debug_bounds("desktop-pet-bubble").is_some());
        pet.update(|_, cx| {
            assert!(
                app.read(cx).encourager.error.is_some(),
                "missing configuration must produce actionable feedback"
            )
        });

        // A slow request keeps thinking until the host reports its completion.
        pet.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.encourager.error = None;
                app.encourager.loading = true;
                cx.notify();
            })
        });
        pet.run_until_parked();
        for _ in 0..8 {
            pet.executor().advance_clock(Duration::from_millis(180));
            pet.run_until_parked();
        }
        pet.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(pet.debug_bounds("desktop-pet-thinking").is_some());
        assert!(pet.debug_bounds("desktop-pet-bubble").is_none());

        // A completed API response follows the same visible feedback path.
        pet.update(|_, cx| {
            app.update(cx, |app, cx| {
                app.encourager.loading = false;
                app.encourager.error = None;
                app.encourager.reply = Some("喵，我在。".into());
                cx.notify();
            })
        });
        pet.executor().advance_clock(Duration::from_millis(180));
        pet.run_until_parked();
        pet.update(|window, cx| {
            let _ = window.draw(cx);
        });
        assert!(pet.debug_bounds("desktop-pet-copy").is_some());
        pet.simulate_keystrokes("escape");
        pet.run_until_parked();
        pet.update(|window, cx| {
            let _ = window.draw(cx);
            assert!(window.painted_quads().is_empty());
        });
        assert!(pet.debug_bounds("desktop-pet-bubble").is_none());
        cx.update(|cx| {
            app.update(cx, |app, cx| {
                app.set_encourager_pet_enabled(false, cx);
                app.persistence.flush();
            })
        });
    }
}
