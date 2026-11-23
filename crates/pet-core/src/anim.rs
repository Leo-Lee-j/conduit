use crate::{EyeKind, Fx, MouthKind, PetState, Pose};
use std::time::{Duration, Instant};

pub struct TickOut {
    pub pose: Pose,
    pub next_wake: Option<Instant>,
}
pub struct Animator {
    state: PetState,
    started: Instant,
    seed: u64,
    blink_at: Instant,
    blinking: bool,
    look_at: Instant,
    look: (i8, i8),
}
impl Animator {
    pub fn new(seed: u64, now: Instant) -> Self {
        let mut this = Self {
            state: PetState::Idle,
            started: now,
            seed: if seed == 0 { 0x9e3779b97f4a7c15 } else { seed },
            blink_at: now,
            blinking: false,
            look_at: now,
            look: (0, 0),
        };
        this.reset(now);
        this
    }
    fn random_ms(&mut self, min: u64, max: u64) -> Duration {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 7;
        self.seed ^= self.seed << 17;
        Duration::from_millis(min + self.seed % (max - min + 1))
    }
    fn reset(&mut self, now: Instant) {
        self.started = now;
        self.blinking = false;
        self.look = (0, 0);
        let min = if self.state == PetState::Working {
            3000
        } else {
            2000
        };
        self.blink_at = now + self.random_ms(min, min + 3000);
        self.look_at = now + self.random_ms(6000, 12000);
    }
    /// Repeated host notifications must not restart an animation.
    pub fn set_state(&mut self, state: PetState, now: Instant) {
        if self.state != state {
            self.state = state;
            self.reset(now);
        }
    }
    pub fn tick(&mut self, now: Instant) -> TickOut {
        let elapsed = now.saturating_duration_since(self.started).as_millis() as u64;
        let mut pose = Pose::default();
        let mut deadlines = Vec::with_capacity(4);
        let mut at = |ms| deadlines.push(self.started + Duration::from_millis(ms));
        match self.state {
            PetState::Idle => {
                pose.bob = ((elapsed / 600) % 2) as u8;
                at((elapsed / 600 + 1) * 600);
            }
            PetState::Working => {
                pose.bob = ((elapsed / 400) % 2) as u8;
                pose.look = (1, -1);
                pose.mouth = MouthKind::Flat;
                pose.fx = Fx::Dots(((elapsed / 400) % 4) as u8);
                at((elapsed / 400 + 1) * 400);
            }
            PetState::Attention => {
                pose.mouth = MouthKind::O;
                let phase = elapsed % 900;
                pose.hop = u8::from((80..160).contains(&phase));
                at(elapsed - phase
                    + if phase < 80 {
                        80
                    } else if phase < 160 {
                        160
                    } else {
                        980
                    });
                if (elapsed / 500).is_multiple_of(2) {
                    pose.fx = Fx::Exclaim { error: false };
                }
                at((elapsed / 500 + 1) * 500);
            }
            PetState::Success | PetState::Poked => {
                pose.eyes = EyeKind::Happy;
                pose.mouth = MouthKind::Open;
                let sequence: &[u8] = if self.state == PetState::Success {
                    &[0, 1, 2, 2, 1, 0, 0, 1, 2, 2, 1, 0]
                } else {
                    &[0, 1, 2, 1, 0]
                };
                let index = (elapsed / 80) as usize;
                pose.hop = sequence.get(index).copied().unwrap_or(0);
                if let Some(next) = sequence
                    .iter()
                    .enumerate()
                    .skip(index + 1)
                    .find(|(_, hop)| **hop != pose.hop)
                {
                    at(next.0 as u64 * 80);
                }
                if self.state == PetState::Success {
                    pose.fx = Fx::Heart((elapsed / 250).min(3) as u8);
                    if elapsed < 750 {
                        at((elapsed / 250 + 1) * 250);
                    }
                }
            }
            PetState::Error => {
                pose.eyes = EyeKind::X;
                pose.mouth = MouthKind::Frown;
                pose.cheeks = false;
                pose.fx = Fx::Exclaim { error: true };
                pose.shake = [-1, 1, -1, 1, 0]
                    .get((elapsed / 60) as usize)
                    .copied()
                    .unwrap_or(0);
                if elapsed < 240 {
                    at((elapsed / 60 + 1) * 60);
                }
            }
            PetState::Sleep => {
                pose.eyes = EyeKind::Half;
                pose.mouth = MouthKind::Flat;
                // One shared 1.4s clock: strictly fewer than one wake per second.
                pose.bob = ((elapsed / 1400) % 2) as u8;
                pose.fx = Fx::Zzz(((elapsed / 1400) % 4) as u8);
                at((elapsed / 1400 + 1) * 1400);
            }
            PetState::Dragged => {
                pose.hop = 1;
                pose.mouth = MouthKind::O;
            }
        }
        if matches!(self.state, PetState::Idle | PetState::Working) {
            // Consume randomized transitions in timestamp order, including after
            // a delayed frame. The pose cannot depend on how often tick is called.
            loop {
                let looking = self.state == PetState::Idle && self.look_at < self.blink_at;
                let next = if looking { self.look_at } else { self.blink_at };
                if next > now {
                    break;
                }
                if looking {
                    if self.look == (0, 0) {
                        let choice = self.random_ms(0, 2).as_millis() as usize;
                        self.look = [(-1, 0), (1, 0), (0, -1)][choice];
                        self.look_at += Duration::from_millis(1200);
                    } else {
                        self.look = (0, 0);
                        let delay = self.random_ms(6000, 12000) - Duration::from_millis(1200);
                        self.look_at += delay;
                    }
                } else {
                    self.blinking = !self.blinking;
                    let delay = if self.blinking {
                        Duration::from_millis(120)
                    } else {
                        let min = if self.state == PetState::Working {
                            3000
                        } else {
                            2000
                        };
                        self.random_ms(min, min + 3000) - Duration::from_millis(120)
                    };
                    self.blink_at += delay;
                }
            }
            if self.blinking {
                pose.eyes = EyeKind::Half;
            }
            deadlines.push(self.blink_at);
        }
        if self.state == PetState::Idle {
            pose.look = self.look;
            deadlines.push(self.look_at);
        }
        TickOut {
            pose,
            next_wake: deadlines.into_iter().filter(|t| *t > now).min(),
        }
    }
}
