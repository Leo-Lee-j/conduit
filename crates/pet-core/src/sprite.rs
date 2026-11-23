pub const CANVAS_W: usize = 25;
pub const CANVAS_H: usize = 19;
pub const BODY: [&str; 12] = [
    "..oooooooooooo..",
    ".o############o.",
    "o##hh##########o",
    "o#h############o",
    "o##############o",
    "o##############o",
    "o##############o",
    "o##############o",
    "o##############o",
    "o##############o",
    ".o############o.",
    "..oooooooooooo..",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EyeKind {
    Open,
    Half,
    Happy,
    X,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouthKind {
    Smile,
    Flat,
    Open,
    O,
    Frown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    None,
    Dots(u8),
    Exclaim { error: bool },
    Heart(u8),
    Zzz(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pose {
    pub bob: u8,
    pub hop: u8,
    pub shake: i8,
    pub eyes: EyeKind,
    pub look: (i8, i8),
    pub mouth: MouthKind,
    pub cheeks: bool,
    pub fx: Fx,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            bob: 0,
            hop: 0,
            shake: 0,
            eyes: EyeKind::Open,
            look: (0, 0),
            mouth: MouthKind::Smile,
            cheeks: true,
            fx: Fx::None,
        }
    }
}

impl EyeKind {
    pub fn cells(self, look: (i8, i8)) -> Vec<(i16, i16)> {
        let points: &[(i16, i16)] = match self {
            Self::Open => &[
                (4, 5),
                (5, 5),
                (4, 6),
                (5, 6),
                (10, 5),
                (11, 5),
                (10, 6),
                (11, 6),
            ],
            Self::Half => &[(4, 6), (5, 6), (10, 6), (11, 6)],
            Self::Happy => &[(3, 6), (4, 5), (5, 6), (10, 6), (11, 5), (12, 6)],
            Self::X => &[
                (3, 5),
                (5, 5),
                (4, 6),
                (3, 7),
                (5, 7),
                (10, 5),
                (12, 5),
                (11, 6),
                (10, 7),
                (12, 7),
            ],
        };
        let (dx, dy) = if matches!(self, Self::Open | Self::Half) {
            (
                i16::from(look.0.clamp(-1, 1)),
                i16::from(look.1.clamp(-1, 0)),
            )
        } else {
            (0, 0)
        };
        points.iter().map(|(x, y)| (x + dx, y + dy)).collect()
    }
}
impl MouthKind {
    pub fn cells(self) -> &'static [(i16, i16)] {
        match self {
            Self::Smile => &[(6, 8), (7, 9), (8, 9), (9, 8)],
            Self::Flat => &[(7, 8), (8, 8)],
            Self::Open => &[(6, 8), (7, 8), (8, 8), (9, 8), (7, 9), (8, 9)],
            Self::O => &[(7, 8), (8, 8), (7, 9), (8, 9)],
            Self::Frown => &[(6, 9), (7, 8), (8, 8), (9, 9)],
        }
    }
}
pub const CHEEKS: [(i16, i16); 4] = [(2, 7), (3, 7), (12, 7), (13, 7)];

/// Packed RRGGBBAA; independent of the renderer's color representation.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub body: u32,
    pub outline: u32,
    pub highlight: u32,
    pub ink: u32,
    pub cheek: u32,
    pub shadow: u32,
    pub halo: u32,
    pub heart: u32,
    pub amber: u32,
    pub red: u32,
}
impl Default for Palette {
    fn default() -> Self {
        Self {
            body: 0xafa9ecff,
            outline: 0x3c3489ff,
            highlight: 0xeeedfeff,
            ink: 0x26215cff,
            cheek: 0xed93b1ff,
            shadow: 0x0000002e,
            halo: 0xffffffff,
            heart: 0xd4537eff,
            amber: 0xef9f27ff,
            red: 0xe24b4aff,
        }
    }
}
