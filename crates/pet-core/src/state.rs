use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PetState {
    #[default]
    Idle,
    Working,
    Attention,
    Success,
    Error,
    Sleep,
    Poked,
    Dragged,
}

#[derive(Clone, Copy, Debug)]
pub enum PetEvent {
    TaskStarted,
    ToolRunning,
    NeedsApproval,
    ApprovalResolved,
    TaskSucceeded,
    TaskFailed,
    Reset,
    DebugState(PetState),
}

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub sleep_after: Duration,
    pub working_timeout: Duration,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            sleep_after: Duration::from_secs(300),
            working_timeout: Duration::from_secs(600),
        }
    }
}

pub struct StateMachine {
    config: Config,
    working: bool,
    attention: bool,
    working_until: Option<Instant>,
    overlay: Option<(PetState, Instant)>,
    debug: Option<(PetState, Instant)>,
    poked_until: Option<Instant>,
    dragging: bool,
    last_activity: Instant,
}
impl StateMachine {
    pub fn new(config: Config, now: Instant) -> Self {
        Self {
            config,
            working: false,
            attention: false,
            working_until: None,
            overlay: None,
            debug: None,
            poked_until: None,
            dragging: false,
            last_activity: now,
        }
    }
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = now;
    }
    pub fn poke(&mut self, now: Instant) {
        self.activity(now);
        self.poked_until = Some(now + Duration::from_millis(600));
    }
    pub fn drag(&mut self, dragging: bool, now: Instant) {
        self.activity(now);
        self.dragging = dragging;
    }
    /// An authoritative in-process snapshot does not expire while the host owns a run.
    /// Calling this with unchanged flags is not user activity and does not reset sleep.
    pub fn sync_activity(&mut self, working: bool, attention: bool, now: Instant) {
        if (self.working, self.attention) != (working, attention) {
            self.activity(now);
            if working && !self.working {
                self.overlay = None;
            }
        }
        self.working = working;
        self.attention = attention;
        self.working_until = None;
    }
    /// Completion feedback may coexist with other runs; it never clears their flags.
    pub fn feedback(&mut self, failed: bool, now: Instant) {
        self.activity(now);
        self.overlay = Some(if failed {
            (PetState::Error, now + Duration::from_secs(8))
        } else {
            (PetState::Success, now + Duration::from_millis(2500))
        });
    }
    pub fn event(&mut self, event: PetEvent, now: Instant) {
        self.activity(now);
        match event {
            PetEvent::TaskStarted | PetEvent::ToolRunning => {
                self.working = true;
                self.working_until = Some(now + self.config.working_timeout);
                self.overlay = None;
            }
            PetEvent::NeedsApproval => self.attention = true,
            PetEvent::ApprovalResolved => self.attention = false,
            PetEvent::TaskSucceeded | PetEvent::TaskFailed => {
                self.working = false;
                self.attention = false;
                self.working_until = None;
                self.feedback(matches!(event, PetEvent::TaskFailed), now);
            }
            PetEvent::Reset => *self = Self::new(self.config, now),
            PetEvent::DebugState(state) => self.debug = Some((state, now + Duration::from_secs(3))),
        }
    }
    pub fn tick(&mut self, now: Instant) -> (PetState, Option<Instant>) {
        if self.working_until.is_some_and(|until| until <= now) {
            self.working = false;
            self.working_until = None;
        }
        if self.overlay.is_some_and(|(_, until)| until <= now) {
            self.overlay = None;
        }
        if self.debug.is_some_and(|(_, until)| until <= now) {
            self.debug = None;
        }
        if self.poked_until.is_some_and(|until| until <= now) {
            self.poked_until = None;
        }
        let sleep_at = self.last_activity + self.config.sleep_after;
        let state = if self.dragging {
            PetState::Dragged
        } else if let Some((state, _)) = self.debug {
            state
        } else if self.poked_until.is_some() {
            PetState::Poked
        } else if self.attention {
            PetState::Attention
        } else if let Some((state, _)) = self.overlay {
            state
        } else if self.working {
            PetState::Working
        } else if now >= sleep_at {
            PetState::Sleep
        } else {
            PetState::Idle
        };
        let next = [
            self.working_until,
            self.overlay.map(|(_, t)| t),
            self.debug.map(|(_, t)| t),
            self.poked_until,
            (state == PetState::Idle).then_some(sleep_at),
        ]
        .into_iter()
        .flatten()
        .filter(|t| *t > now)
        .min();
        (state, next)
    }
}
