//! Deterministic desktop pet logic. No UI, OS, I/O, or clock reads.
pub mod anim;
pub mod raster;
pub mod sprite;
pub mod state;

pub use anim::{Animator, TickOut};
pub use raster::{Frame, Rect, rasterize};
pub use sprite::{CANVAS_H, CANVAS_W, EyeKind, Fx, MouthKind, Palette, Pose};
pub use state::{Config, PetEvent, PetState, StateMachine};
