//! Double-Shift handling and the file category of workspace search.
use super::*;

const TAP_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub(super) struct DoubleShift {
    modifiers: Modifiers,
    pressed: Option<Instant>,
    released: Option<Instant>,
}

impl DoubleShift {
    pub(super) fn cancel(&mut self) {
        self.pressed = None;
        self.released = None;
    }

    pub(super) fn update(&mut self, modifiers: Modifiers, now: Instant) -> bool {
        if modifiers == self.modifiers {
            return false;
        }
        let previous = std::mem::replace(&mut self.modifiers, modifiers);
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        if modifiers == shift && previous == Modifiers::default() {
            self.pressed = Some(now);
        } else if modifiers == Modifiers::default() && previous == shift {
            let pressed = self.pressed.take();
            if pressed.is_some_and(|time| now.duration_since(time) <= TAP_INTERVAL) {
                if self
                    .released
                    .take()
                    .is_some_and(|time| now.duration_since(time) <= TAP_INTERVAL)
                {
                    return true;
                }
                self.released = Some(now);
            } else {
                self.cancel();
            }
        } else {
            // Ctrl/Alt/Win/Fn combinations are not standalone Shift taps.
            self.cancel();
        }
        false
    }
}

impl ConduitApp {
    pub(super) fn open_file_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_workspace_search(true, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{DoubleShift, Duration, Instant, Modifiers};
    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::default()
        }
    }
    fn tap(state: &mut DoubleShift, time: Instant) -> bool {
        assert!(!state.update(shift(), time));
        state.update(Modifiers::default(), time + Duration::from_millis(40))
    }

    #[test]
    fn double_shift_requires_two_complete_taps() {
        let _locale = crate::test_support::chinese();
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!tap(&mut state, now));
        assert!(tap(&mut state, now + Duration::from_millis(150)));
        assert!(
            !tap(&mut state, now + Duration::from_millis(300)),
            "do not fire again on the third tap"
        );
    }

    #[test]
    fn double_shift_ignores_repeats_holds_and_slow_taps() {
        let _locale = crate::test_support::chinese();
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!state.update(shift(), now));
        assert!(!state.update(shift(), now + Duration::from_millis(50)));
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(600)));
        assert!(!tap(&mut state, now + Duration::from_millis(700)));
        assert!(!tap(&mut state, now + Duration::from_millis(1400)));
        assert!(tap(&mut state, now + Duration::from_millis(1550)));
    }

    #[test]
    fn double_shift_cancels_on_typing_and_other_modifiers() {
        let _locale = crate::test_support::chinese();
        let now = Instant::now();
        let mut state = DoubleShift::default();
        assert!(!tap(&mut state, now));
        state.cancel();
        assert!(!tap(&mut state, now + Duration::from_millis(100)));
        assert!(!state.update(shift(), now + Duration::from_millis(200)));
        state.cancel(); // Shift+letter or Shift+Enter.
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(240)));
        assert!(!tap(&mut state, now + Duration::from_millis(300)));
        assert!(!state.update(
            Modifiers {
                control: true,
                ..shift()
            },
            now + Duration::from_millis(350)
        ));
        assert!(!state.update(shift(), now + Duration::from_millis(360)));
        assert!(!state.update(Modifiers::default(), now + Duration::from_millis(400)));
        assert!(!tap(&mut state, now + Duration::from_millis(450)));
    }
}
